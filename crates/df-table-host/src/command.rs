//! The text of the developer command line, parsed into typed commands.
//!
//! It owns the grammar and the usage text, and nothing else: no sockets, no files, no printing.
//! Values stay text here because only the running canvas knows each field's type, so a spawn or a
//! set is sent with `ValueForm::Text` and the canvas reads each value by its field's type.

use std::collections::BTreeSet;
use std::fmt;
use std::path::PathBuf;

use df_playground::controls::{ControlsChange, Device};
use df_playground::copy::ReadingPace;
use df_playground::fixture::{Choice, PartyMember, UnavailableChoice};
use df_playground::scene::SceneChange;

use crate::wire::{EffectRequest, ValueForm};

const SPAWN_USAGE: &str = "spawn <kind> <id> [x=N] [y=N] [field=value ...]";
const SET_USAGE: &str = "set <id> <field>=<value> [<field>=<value> ...]";
const MOVE_USAGE: &str = "move <id> <x> <y>";
const DESPAWN_USAGE: &str = "despawn <id> [<id> ...]";
const EVENTS_USAGE: &str = "events [after]";
const SCRIPT_USAGE: &str = "script <file>";
const PLAYGROUND_USAGE: &str = "playground [set <key>=<value> ...]";
const SCENE_USAGE: &str =
    "scene caption <text> [speaker=NAME] | caption-clear | party ... | choices ... | reset";
const CAPTION_USAGE: &str = "scene caption <text> [speaker=NAME]";
const PARTY_USAGE: &str = "scene party add <name> hp=N [max=N] [color=#rrggbb] [conditions=a,b] | set <name> [hp=N] [max=N] [conditions=a,b] | remove <name> | clear";
const CHOICES_USAGE: &str = "scene choices suggested [<label>[=<explanation>] ...] | group <name> [<label> ...] | unavailable [<label>=<reason> ...] | clear";
const FIXTURE_USAGE: &str =
    "fixture show <name> | fixture write <name> <file> | fixture remove <name>";
/// The keys `playground set` accepts and what each takes.
const PLAYGROUND_KEYS: &str = "fixture=<name>, device=both|tv|phone, pace=slow|normal|fast, status=fixture|<status>, unavailable=true|false, grid=true|false";

/// A command for the running canvas.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// One or more effects, sent in order.
    Effects(Vec<EffectRequest>),
    Canvas,
    Kinds,
    /// Events newer than the sequence, or the newest page when none is given.
    Events {
        after: Option<u64>,
    },
    Reload,
    /// Print what the playground is showing and the fixtures it can show.
    Playground,
    /// Change what the playground shows, in order.
    PlaygroundSet(Vec<ControlsChange>),
    /// One live change to the caption, the party or the phone's choices.
    Scene(SceneChange),
    /// The names of the fixture files.
    Fixtures,
    FixtureShow(String),
    /// Write the file's text as the named fixture.
    FixtureWrite {
        name: String,
        path: PathBuf,
    },
    FixtureRemove(String),
    /// A file of commands, one per line.
    Script(PathBuf),
    Help,
}

/// Why a command line was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandError {
    Empty,
    Unknown(String),
    Usage(&'static str),
    BadNumber {
        what: &'static str,
        text: String,
    },
    BadPair(String),
    DuplicateKey(String),
    /// A playground key that is not one of the keys, or a value that is not one of its choices.
    BadChoice {
        key: String,
        text: String,
        allowed: &'static str,
    },
    /// A hit-point or similar count that is not a whole number from 0.
    BadCount {
        what: &'static str,
        text: String,
    },
    UnclosedQuote,
    /// `script` and `help` mean nothing inside a script.
    NotAllowedInScript(&'static str),
}

impl fmt::Display for CommandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("no command given (try: help)"),
            Self::Unknown(word) => write!(formatter, "unknown command '{word}' (try: help)"),
            Self::Usage(usage) => write!(formatter, "usage: {usage}"),
            Self::BadNumber { what, text } => {
                write!(formatter, "{what} must be a finite number, not '{text}'")
            }
            Self::BadPair(text) => write!(formatter, "expected field=value, not '{text}'"),
            Self::DuplicateKey(key) => write!(formatter, "'{key}' is given twice"),
            Self::BadChoice { key, text, allowed } => {
                write!(
                    formatter,
                    "'{text}' is not a choice for {key}; use {allowed}"
                )
            }
            Self::BadCount { what, text } => {
                write!(
                    formatter,
                    "{what} must be a whole number from 0, not '{text}'"
                )
            }
            Self::UnclosedQuote => formatter.write_str("a quote is not closed"),
            Self::NotAllowedInScript(word) => {
                write!(formatter, "'{word}' cannot be used inside a script")
            }
        }
    }
}

/// The text printed by `help`.
pub fn usage_text() -> String {
    format!(
        "df-table dev [--port N] <command>\n\
         \n  {SPAWN_USAGE}\n      put an element on the canvas (x and y are its position; a field\n      called x or y can be changed with set)\n\
         \n  {SET_USAGE}\n      change fields of an element\n\
         \n  {MOVE_USAGE}\n      move an element (x 0 to 100, y 0 to 56.25)\n\
         \n  {DESPAWN_USAGE}\n      remove elements\n\
         \n  clear\n      remove every element\n\
         \n  canvas\n      print the canvas as JSON\n\
         \n  kinds\n      list the kinds and their fields\n\
         \n  {EVENTS_USAGE}\n      print the newest events, or those after a sequence\n\
         \n  reload\n      re-read the kind files without a restart\n\
         \n  {PLAYGROUND_USAGE}\n      show or change what the playground page shows; keys: {PLAYGROUND_KEYS}\n\
         \n  {SCENE_USAGE}\n      change the playground's caption, party strip or phone choices live;\n      {PARTY_USAGE}\n      {CHOICES_USAGE}\n      a fixture is not changed; scene reset or another fixture goes back to the file\n\
         \n  fixtures\n      list the playground's fixture files\n\
         \n  {FIXTURE_USAGE}\n      read, write (checked first) or delete a fixture file\n\
         \n  {SCRIPT_USAGE}\n      run a file of commands, one per line; a line may start with #\n\
         \n  help\n      this text\n\
         \nQuote a value with spaces: name=\"Pip the Brave\".\n\
         Several effects in one command are applied in order and every failure is reported;\n\
         a script stops at its first failing line.\n\
         The port is --port N, or the DF_TABLE_PORT environment variable, or 7878."
    )
}

/// Splits a line into words, keeping text inside double quotes together.
pub fn split_words(line: &str) -> Result<Vec<String>, CommandError> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut in_word = false;
    let mut in_quotes = false;
    for character in line.chars() {
        match character {
            '"' => {
                in_quotes = !in_quotes;
                in_word = true;
            }
            character if character.is_whitespace() && !in_quotes => {
                if in_word {
                    words.push(std::mem::take(&mut current));
                    in_word = false;
                }
            }
            other => {
                current.push(other);
                in_word = true;
            }
        }
    }
    if in_quotes {
        return Err(CommandError::UnclosedQuote);
    }
    if in_word {
        words.push(current);
    }
    Ok(words)
}

fn pair(text: &str) -> Result<(&str, &str), CommandError> {
    text.split_once('=')
        .filter(|(name, _)| !name.is_empty())
        .ok_or_else(|| CommandError::BadPair(text.to_owned()))
}

fn number(what: &'static str, text: &str) -> Result<f32, CommandError> {
    text.parse::<f32>()
        .ok()
        .filter(|value| value.is_finite())
        .ok_or_else(|| CommandError::BadNumber {
            what,
            text: text.to_owned(),
        })
}

fn spawn(words: &[String]) -> Result<Command, CommandError> {
    let [kind, id, pairs @ ..] = words else {
        return Err(CommandError::Usage(SPAWN_USAGE));
    };
    let (mut x, mut y) = (None, None);
    let mut fields = serde_json::Map::new();
    let mut seen = BTreeSet::new();
    for text in pairs {
        let (name, value) = pair(text)?;
        if !seen.insert(name) {
            return Err(CommandError::DuplicateKey(name.to_owned()));
        }
        match name {
            "x" => x = Some(number("x", value)?),
            "y" => y = Some(number("y", value)?),
            _ => {
                fields.insert(name.to_owned(), serde_json::Value::String(value.to_owned()));
            }
        }
    }
    Ok(Command::Effects(vec![EffectRequest::Spawn {
        id: id.clone(),
        kind: kind.clone(),
        x,
        y,
        fields,
        values: ValueForm::Text,
    }]))
}

fn set(words: &[String]) -> Result<Command, CommandError> {
    let [id, pairs @ ..] = words else {
        return Err(CommandError::Usage(SET_USAGE));
    };
    if pairs.is_empty() {
        return Err(CommandError::Usage(SET_USAGE));
    }
    pairs
        .iter()
        .map(|text| {
            let (field, value) = pair(text)?;
            Ok(EffectRequest::Set {
                id: id.clone(),
                field: field.to_owned(),
                value: serde_json::Value::String(value.to_owned()),
                values: ValueForm::Text,
            })
        })
        .collect::<Result<Vec<_>, CommandError>>()
        .map(Command::Effects)
}

fn move_element(words: &[String]) -> Result<Command, CommandError> {
    let [id, x, y] = words else {
        return Err(CommandError::Usage(MOVE_USAGE));
    };
    Ok(Command::Effects(vec![EffectRequest::Move {
        id: id.clone(),
        x: number("x", x)?,
        y: number("y", y)?,
    }]))
}

fn events(words: &[String]) -> Result<Command, CommandError> {
    match words {
        [] => Ok(Command::Events { after: None }),
        [after] => after
            .parse::<u64>()
            .map(|after| Command::Events { after: Some(after) })
            .map_err(|_| CommandError::BadNumber {
                what: "the events sequence",
                text: after.clone(),
            }),
        _ => Err(CommandError::Usage(EVENTS_USAGE)),
    }
}

fn count(what: &'static str, text: &str) -> Result<u32, CommandError> {
    text.parse::<u32>().map_err(|_| CommandError::BadCount {
        what,
        text: text.to_owned(),
    })
}

/// Reads `key=value` words, refusing a key given twice and any key not in `allowed`.
fn keyed<'a>(
    words: &'a [String],
    allowed: &'static str,
    keys: &[&str],
) -> Result<Vec<(&'a str, &'a str)>, CommandError> {
    let mut seen = BTreeSet::new();
    let mut pairs = Vec::new();
    for text in words {
        let (key, value) = pair(text)?;
        if !keys.contains(&key) {
            return Err(CommandError::BadChoice {
                key: "this command".to_owned(),
                text: key.to_owned(),
                allowed,
            });
        }
        if !seen.insert(key) {
            return Err(CommandError::DuplicateKey(key.to_owned()));
        }
        pairs.push((key, value));
    }
    Ok(pairs)
}

fn split_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_owned)
        .collect()
}

fn caption(words: &[String]) -> Result<Command, CommandError> {
    let [text, rest @ ..] = words else {
        return Err(CommandError::Usage(CAPTION_USAGE));
    };
    let mut speaker = None;
    for (_, value) in keyed(rest, "speaker=NAME", &["speaker"])? {
        speaker = Some(value.to_owned());
    }
    Ok(Command::Scene(SceneChange::Caption {
        speaker,
        text: text.clone(),
    }))
}

fn party(words: &[String]) -> Result<Command, CommandError> {
    const MEMBER_KEYS: [&str; 4] = ["hp", "max", "color", "conditions"];
    match words {
        [verb] if verb == "clear" => Ok(Command::Scene(SceneChange::PartyClear {})),
        [verb, name] if verb == "remove" => Ok(Command::Scene(SceneChange::PartyRemove {
            name: name.clone(),
        })),
        [verb, name, pairs @ ..] if verb == "add" => {
            let mut member = PartyMember {
                name: name.clone(),
                hit_points: 0,
                maximum_hit_points: 0,
                conditions: Vec::new(),
                color: None,
            };
            let mut has_hit_points = false;
            let mut has_maximum = false;
            for (key, value) in keyed(
                pairs,
                "hp=N, max=N, color=#rrggbb, conditions=a,b",
                &MEMBER_KEYS,
            )? {
                match key {
                    "hp" => {
                        member.hit_points = count("hp", value)?;
                        has_hit_points = true;
                    }
                    "max" => {
                        member.maximum_hit_points = count("max", value)?;
                        has_maximum = true;
                    }
                    "color" => member.color = Some(value.to_owned()),
                    _ => member.conditions = split_list(value),
                }
            }
            if !has_hit_points {
                return Err(CommandError::Usage(PARTY_USAGE));
            }
            if !has_maximum {
                member.maximum_hit_points = member.hit_points;
            }
            Ok(Command::Scene(SceneChange::PartyAdd { member }))
        }
        [verb, name, pairs @ ..] if verb == "set" && !pairs.is_empty() => {
            let (mut hit_points, mut maximum_hit_points, mut conditions) = (None, None, None);
            for (key, value) in keyed(
                pairs,
                "hp=N, max=N, conditions=a,b",
                &["hp", "max", "conditions"],
            )? {
                match key {
                    "hp" => hit_points = Some(count("hp", value)?),
                    "max" => maximum_hit_points = Some(count("max", value)?),
                    _ => conditions = Some(split_list(value)),
                }
            }
            Ok(Command::Scene(SceneChange::PartySet {
                name: name.clone(),
                hit_points,
                maximum_hit_points,
                conditions,
            }))
        }
        _ => Err(CommandError::Usage(PARTY_USAGE)),
    }
}

/// A choice as `label` or `label=explanation`.
fn choice(text: &str) -> Choice {
    match text.split_once('=') {
        Some((label, explain)) => Choice {
            label: label.to_owned(),
            explain: Some(explain.to_owned()),
        },
        None => Choice {
            label: text.to_owned(),
            explain: None,
        },
    }
}

fn choices(words: &[String]) -> Result<Command, CommandError> {
    match words {
        [verb] if verb == "clear" => Ok(Command::Scene(SceneChange::ClearChoices {})),
        [verb, labels @ ..] if verb == "suggested" => Ok(Command::Scene(SceneChange::Suggested {
            choices: labels.iter().map(|text| choice(text)).collect(),
        })),
        [verb, name, labels @ ..] if verb == "group" => Ok(Command::Scene(SceneChange::Group {
            name: name.clone(),
            choices: labels.iter().map(|text| choice(text)).collect(),
        })),
        [verb, entries @ ..] if verb == "unavailable" => {
            let mut choices = Vec::new();
            for text in entries {
                let (label, reason) = pair(text)?;
                choices.push(UnavailableChoice {
                    label: label.to_owned(),
                    reason: reason.to_owned(),
                });
            }
            Ok(Command::Scene(SceneChange::Unavailable { choices }))
        }
        _ => Err(CommandError::Usage(CHOICES_USAGE)),
    }
}

fn scene(words: &[String]) -> Result<Command, CommandError> {
    match words.split_first() {
        Some((verb, rest)) if verb == "caption" => caption(rest),
        Some((verb, rest)) if verb == "caption-clear" && rest.is_empty() => {
            Ok(Command::Scene(SceneChange::ClearCaption {}))
        }
        Some((verb, rest)) if verb == "party" => party(rest),
        Some((verb, rest)) if verb == "choices" => choices(rest),
        Some((verb, rest)) if verb == "reset" && rest.is_empty() => {
            Ok(Command::Scene(SceneChange::Reset {}))
        }
        _ => Err(CommandError::Usage(SCENE_USAGE)),
    }
}

fn truth(key: &str, text: &str) -> Result<bool, CommandError> {
    match text {
        "true" | "on" => Ok(true),
        "false" | "off" => Ok(false),
        _ => Err(CommandError::BadChoice {
            key: key.to_owned(),
            text: text.to_owned(),
            allowed: "true or false",
        }),
    }
}

fn playground_change(key: &str, value: &str) -> Result<ControlsChange, CommandError> {
    match key {
        "fixture" => Ok(ControlsChange::Fixture(value.to_owned())),
        "device" => Device::from_key(value)
            .map(ControlsChange::Device)
            .ok_or_else(|| CommandError::BadChoice {
                key: key.to_owned(),
                text: value.to_owned(),
                allowed: "both, tv or phone",
            }),
        "pace" => ReadingPace::from_key(value)
            .map(ControlsChange::Pace)
            .ok_or_else(|| CommandError::BadChoice {
                key: key.to_owned(),
                text: value.to_owned(),
                allowed: "slow, normal or fast",
            }),
        "status" => Ok(ControlsChange::Status(value.to_owned())),
        "unavailable" => truth(key, value).map(ControlsChange::ShowUnavailable),
        "grid" => truth(key, value).map(ControlsChange::ShowGrid),
        _ => Err(CommandError::BadChoice {
            key: "playground set".to_owned(),
            text: key.to_owned(),
            allowed: PLAYGROUND_KEYS,
        }),
    }
}

fn playground(words: &[String]) -> Result<Command, CommandError> {
    match words {
        [] => Ok(Command::Playground),
        [word, pairs @ ..] if word == "set" && !pairs.is_empty() => {
            let mut seen = BTreeSet::new();
            let mut changes = Vec::new();
            for text in pairs {
                let (key, value) = pair(text)?;
                if !seen.insert(key) {
                    return Err(CommandError::DuplicateKey(key.to_owned()));
                }
                changes.push(playground_change(key, value)?);
            }
            Ok(Command::PlaygroundSet(changes))
        }
        _ => Err(CommandError::Usage(PLAYGROUND_USAGE)),
    }
}

fn fixture(words: &[String]) -> Result<Command, CommandError> {
    match words {
        [verb, name] if verb == "show" => Ok(Command::FixtureShow(name.clone())),
        [verb, name] if verb == "remove" => Ok(Command::FixtureRemove(name.clone())),
        [verb, name, path] if verb == "write" => Ok(Command::FixtureWrite {
            name: name.clone(),
            path: PathBuf::from(path),
        }),
        _ => Err(CommandError::Usage(FIXTURE_USAGE)),
    }
}

/// Parses one command line, already split into words.
pub fn parse(words: &[String]) -> Result<Command, CommandError> {
    let (name, rest) = words.split_first().ok_or(CommandError::Empty)?;
    match name.as_str() {
        "spawn" => spawn(rest),
        "set" => set(rest),
        "move" => move_element(rest),
        "despawn" => {
            if rest.is_empty() {
                return Err(CommandError::Usage(DESPAWN_USAGE));
            }
            Ok(Command::Effects(
                rest.iter()
                    .map(|id| EffectRequest::Despawn { id: id.clone() })
                    .collect(),
            ))
        }
        "clear" => Ok(Command::Effects(vec![EffectRequest::Clear {}])),
        "canvas" => Ok(Command::Canvas),
        "kinds" => Ok(Command::Kinds),
        "events" => events(rest),
        "reload" => Ok(Command::Reload),
        "playground" => playground(rest),
        "scene" => scene(rest),
        "fixtures" => {
            if rest.is_empty() {
                Ok(Command::Fixtures)
            } else {
                Err(CommandError::Usage("fixtures"))
            }
        }
        "fixture" => fixture(rest),
        "script" => match rest {
            [path] => Ok(Command::Script(PathBuf::from(path))),
            _ => Err(CommandError::Usage(SCRIPT_USAGE)),
        },
        "help" => Ok(Command::Help),
        other => Err(CommandError::Unknown(other.to_owned())),
    }
}

/// Parses one line of a script. A script drives the running host, so `script` and `help` are
/// refused here, when the file is read, and not after earlier lines have already run.
pub fn parse_script_line(words: &[String]) -> Result<Command, CommandError> {
    match parse(words)? {
        Command::Script(_) => Err(CommandError::NotAllowedInScript("script")),
        Command::Help => Err(CommandError::NotAllowedInScript("help")),
        command => Ok(command),
    }
}
