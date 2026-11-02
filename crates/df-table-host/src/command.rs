//! The text of the developer command line, parsed into typed commands.
//!
//! It owns the grammar and the usage text, and nothing else: no sockets, no files, no printing.
//! Values stay text here because only the running canvas knows each field's type, so a spawn or a
//! set is sent with `ValueForm::Text` and the canvas reads each value by its field's type.

use std::collections::BTreeSet;
use std::fmt;
use std::path::PathBuf;

use crate::wire::{EffectRequest, ValueForm};

const SPAWN_USAGE: &str = "spawn <kind> <id> [x=N] [y=N] [field=value ...]";
const SET_USAGE: &str = "set <id> <field>=<value> [<field>=<value> ...]";
const MOVE_USAGE: &str = "move <id> <x> <y>";
const DESPAWN_USAGE: &str = "despawn <id> [<id> ...]";
const EVENTS_USAGE: &str = "events [after]";
const SCRIPT_USAGE: &str = "script <file>";

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
        "script" => match rest {
            [path] => Ok(Command::Script(PathBuf::from(path))),
            _ => Err(CommandError::Usage(SCRIPT_USAGE)),
        },
        "help" => Ok(Command::Help),
        other => Err(CommandError::Unknown(other.to_owned())),
    }
}

/// Parses one line of a script. A script runs against the canvas only, so `script` and `help` are
/// refused here, when the file is read, and not after earlier lines have already run.
pub fn parse_script_line(words: &[String]) -> Result<Command, CommandError> {
    match parse(words)? {
        Command::Script(_) => Err(CommandError::NotAllowedInScript("script")),
        Command::Help => Err(CommandError::NotAllowedInScript("help")),
        command => Ok(command),
    }
}
