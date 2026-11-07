//! The text of the developer command line, parsed into typed commands.
//!
//! It owns the grammar and the usage text, and nothing else: no sockets, no files, no printing.
//! Values stay text here because only the running canvas knows each field's type, so a spawn or a
//! set is sent with `ValueForm::Text` and the canvas reads each value by its field's type.
//!
//! The verbs about elements are here; the `scene` verbs are in `scene`, the `playground`, `media`
//! and `fixture` verbs in `playground`, and the usage lines and the help in `usage`.

mod host;
mod media_generate;
mod playground;
mod scene;
mod usage;

use std::collections::BTreeSet;
use std::fmt;
use std::path::PathBuf;

use df_playground::controls::ControlsChange;
use df_playground::scene::SceneChange;
use df_table::element::{Degrees, Percent};

use crate::wire::{EffectRequest, PivotRequest, ValueForm};

pub use self::host::{EventsRead, TimelineRead};
pub use self::media_generate::MediaGenerate;
pub use self::playground::{DEFAULT_CYCLES, DEFAULT_HOLD_MS};
pub use self::scene::{DEFAULT_CAMERA_MS, DEFAULT_SLIDE, DEFAULT_VOLUME, DEFAULT_ZOOM_PERCENT};
pub use self::usage::usage_text;
use self::usage::{
    DESPAWN_USAGE, FIXTURES_USAGE, MOVE_USAGE, REPL_USAGE, ROTATE_USAGE, SCRIPT_USAGE, SET_USAGE,
    SPAWN_USAGE, STOP_USAGE, TRAVEL_USAGE, WAIT_USAGE,
};
pub use crate::locate::MediaLocate;
use crate::observe::EventLevel;

/// Canvas units per second when `travel` is given no speed: the canvas in about five seconds.
pub const DEFAULT_TRAVEL_SPEED: f32 = 20.0;

/// The top-level verbs `parse` matches by name. The verbs of `engine::VERBS`,
/// `scene_builder::cli::VERBS` and `director::cli::VERBS` are matched after these, so no verb may
/// be in two of the tables.
pub const VERBS: [&str; 27] = [
    "spawn",
    "set",
    "move",
    "travel",
    "rotate",
    "stop",
    "repl",
    "despawn",
    "clear",
    "canvas",
    "kinds",
    "events",
    "reload",
    "playground",
    "scene",
    "wait",
    "media",
    "clients",
    "host",
    "frame",
    "services",
    "persona",
    "speech",
    "fixtures",
    "fixture",
    "script",
    "help",
];

/// A command for the running canvas.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// One or more effects, sent in order.
    Effects(Vec<EffectRequest>),
    Canvas,
    Kinds,
    /// Events newer than the sequence, of one area, at most a number; the newest page when no
    /// sequence is given.
    Events(EventsRead),
    /// Read the level of the host's event sinks, or set it.
    EventsLevel(Option<EventLevel>),
    /// What the host's log, event sinks and clock are doing.
    HostStatus,
    /// The playground's frame: its count, run, size and top-level keys. With `after`, wait for the
    /// frame after that count.
    Frame {
        after: Option<u64>,
    },
    Reload,
    /// Read commands from standard input until it ends, one per line, keeping the connection
    /// to the host. Only the command line starts it.
    Repl,
    /// Print what the playground is showing and the fixtures it can show.
    Playground,
    /// Change what the playground shows, one change at a time in order. A refused change is
    /// reported and the others still apply.
    PlaygroundSet(Vec<ControlsChange>),
    /// Go to a scene: the transition settings and the fixture, last. The host applies all of them
    /// or none, as one numbered change.
    PlaygroundGo(Vec<ControlsChange>),
    /// Go to `first`, hold, go to `second`, hold, and so on for `times` rounds. The pauses are
    /// kept by the command line, not the host, and there is none after the last scene.
    PlaygroundCycle {
        first: String,
        second: String,
        times: u32,
        hold_ms: u64,
        transition: Vec<ControlsChange>,
    },
    /// Pause the command line, for a script or a session that stages scenes over time.
    Wait(u64),
    /// One live change to the caption, the party, the phone's choices, pictures or sound.
    Scene(SceneChange),
    /// List the media library.
    Media,
    /// Add a local file to the media library, under its own name or `name`.
    MediaAdd {
        path: PathBuf,
        name: Option<String>,
    },
    MediaRemove(String),
    /// The status and headers the media file's route answers a HEAD with.
    MediaHead(String),
    /// Make one picture from a prompt through the image service, or list the generations.
    MediaGenerate(MediaGenerate),
    /// Scan a picture with SAM through the imagery API, show its layout or name a region.
    MediaLocate(MediaLocate),
    /// The characters of the shown scene: list them, cue one, or frame one.
    Cast(crate::cast::CastCommand),
    /// The personas of characters: list, show, write, set a field or remove.
    Persona(crate::persona::PersonaCommand),
    /// A scene transition's stages from a movement line: `move say|interpret|design|brief`.
    Move(crate::movement::cli::MoveCommand),
    /// Voice chat: the speech service's status, a spoken line, a transcript, voices and clips.
    Speech(crate::speech::SpeechCommand),
    /// What each open page says it is showing and playing.
    Clients,
    /// What one open page says it is showing and playing.
    ClientShow(String),
    /// What one page showed and played, and when.
    ClientsTimeline(TimelineRead),
    /// How the media and text services are set up and how they stand.
    Services(crate::services::Services),
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
    /// A verb of the owner actor: `input`, `viable`, `dump`, `force`, `projection`, `queue`,
    /// `describe` or `broker`.
    Engine(crate::engine::EngineCommand),
    /// A verb of the scene builder's workbench: `scene build`, `scene show`, `plates` or
    /// `buffer`.
    SceneBuilder(crate::scene_builder::SceneBuilderCommand),
    /// A verb of the Director: `camera focus|wide` or `director ...`.
    Director(crate::director::DirectorCommand),
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
        allowed: String,
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

/// A value that is not one of the choices of `key`; `allowed` says what is.
pub(crate) fn bad_choice(key: &str, text: &str, allowed: impl Into<String>) -> CommandError {
    CommandError::BadChoice {
        key: key.to_owned(),
        text: text.to_owned(),
        allowed: allowed.into(),
    }
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

fn travel(words: &[String]) -> Result<Command, CommandError> {
    let [id, x, y, rest @ ..] = words else {
        return Err(CommandError::Usage(TRAVEL_USAGE));
    };
    let mut speed = DEFAULT_TRAVEL_SPEED;
    for (_, value) in keyed(rest, "speed=N", &["speed"])? {
        speed = number("speed", value)?;
    }
    Ok(Command::Effects(vec![EffectRequest::Travel {
        id: id.clone(),
        x: number("x", x)?,
        y: number("y", y)?,
        speed,
    }]))
}

const ROTATION_RANGE: &str =
    "a whole number of degrees from 0 to 359 clockwise (180 is upside down)";
const PIVOT_RANGE: &str = concat!(
    "two whole numbers of percent from 0 to 100 as X,Y ",
    "(50,50 is the centre, 0,50 a hinge on the left edge)"
);

fn rotate(words: &[String]) -> Result<Command, CommandError> {
    let [id, degrees, rest @ ..] = words else {
        return Err(CommandError::Usage(ROTATE_USAGE));
    };
    let rotation_degrees = degrees
        .parse::<i64>()
        .ok()
        .filter(|degrees| Degrees::new(*degrees).is_ok())
        .ok_or_else(|| bad_choice("degrees", degrees, ROTATION_RANGE))?;
    let mut pivot = None;
    for (_, value) in keyed(rest, "pivot=X,Y", &["pivot"])? {
        pivot = Some(pivot_request(value)?);
    }
    Ok(Command::Effects(vec![EffectRequest::Transform {
        id: id.clone(),
        rotation_degrees: serde_json::Number::from(rotation_degrees),
        pivot,
    }]))
}

fn pivot_request(value: &str) -> Result<PivotRequest, CommandError> {
    let bad_pivot = || bad_choice("pivot", value, PIVOT_RANGE);
    let (x, y) = value.split_once(',').ok_or_else(bad_pivot)?;
    let share = |text: &str| {
        text.parse::<i64>()
            .ok()
            .filter(|percent| Percent::new(*percent).is_ok())
            .ok_or_else(bad_pivot)
    };
    Ok(PivotRequest {
        x_percent: serde_json::Number::from(share(x)?),
        y_percent: serde_json::Number::from(share(y)?),
    })
}

/// Splits `words` into those without an `=` (positional) and those with one (`key=value`).
fn split_pair_words(words: &[String]) -> (Vec<String>, Vec<String>) {
    words.iter().cloned().partition(|word| !word.contains('='))
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
            return Err(bad_choice("this command", key, allowed));
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

fn truth(key: &str, text: &str) -> Result<bool, CommandError> {
    match text {
        "true" | "on" => Ok(true),
        "false" | "off" => Ok(false),
        _ => Err(bad_choice(key, text, "true or false")),
    }
}

/// Every id after the verb as one effect each, or the usage line when there are none.
fn each_id(
    ids: &[String],
    usage: &'static str,
    effect: impl Fn(&str) -> EffectRequest,
) -> Result<Command, CommandError> {
    if ids.is_empty() {
        return Err(CommandError::Usage(usage));
    }
    Ok(Command::Effects(ids.iter().map(|id| effect(id)).collect()))
}

/// A command that takes no words: the usage line, which is just its name, when any are given.
fn without_words(
    words: &[String],
    usage: &'static str,
    command: Command,
) -> Result<Command, CommandError> {
    if words.is_empty() {
        Ok(command)
    } else {
        Err(CommandError::Usage(usage))
    }
}

/// Parses one command line, already split into words.
pub fn parse(words: &[String]) -> Result<Command, CommandError> {
    let (name, rest) = words.split_first().ok_or(CommandError::Empty)?;
    match name.as_str() {
        "spawn" => spawn(rest),
        "set" => set(rest),
        "move" => match rest.first().map(String::as_str) {
            Some("say" | "act" | "interpret" | "design" | "brief") => {
                crate::movement::cli::parse(rest).map(Command::Move)
            }
            _ => move_element(rest),
        },
        "travel" => travel(rest),
        "rotate" => rotate(rest),
        "stop" => each_id(rest, STOP_USAGE, |id| EffectRequest::Stop {
            id: id.to_owned(),
        }),
        "repl" => without_words(rest, REPL_USAGE, Command::Repl),
        "despawn" => each_id(rest, DESPAWN_USAGE, |id| EffectRequest::Despawn {
            id: id.to_owned(),
        }),
        "clear" => Ok(Command::Effects(vec![EffectRequest::Clear {}])),
        "canvas" => Ok(Command::Canvas),
        "kinds" => Ok(Command::Kinds),
        "events" => host::events(rest),
        "host" => host::host(rest),
        "frame" => host::frame(rest),
        "services" => crate::services::parse(rest).map(Command::Services),
        "persona" => crate::persona::parse(rest).map(Command::Persona),
        "speech" => crate::speech::parse(rest).map(Command::Speech),
        "reload" => Ok(Command::Reload),
        "playground" => playground::playground(rest),
        "scene" => scene::scene(rest),
        "wait" => match rest {
            [text] => playground::wait_milliseconds("wait", text).map(Command::Wait),
            _ => Err(CommandError::Usage(WAIT_USAGE)),
        },
        "media" => playground::media(rest),
        "clients" => host::clients(rest),
        "fixtures" => without_words(rest, FIXTURES_USAGE, Command::Fixtures),
        "fixture" => playground::fixture(rest),
        "script" => match rest {
            [path] => Ok(Command::Script(PathBuf::from(path))),
            _ => Err(CommandError::Usage(SCRIPT_USAGE)),
        },
        "help" => Ok(Command::Help),
        verb if crate::engine::VERBS.contains(&verb) => {
            crate::engine::parse_verb(verb, rest).map(Command::Engine)
        }
        verb if crate::scene_builder::cli::VERBS.contains(&verb) => {
            crate::scene_builder::parse(verb, rest).map(Command::SceneBuilder)
        }
        verb if crate::director::cli::VERBS.contains(&verb) => {
            crate::director::parse(verb, rest).map(Command::Director)
        }
        other => Err(CommandError::Unknown(other.to_owned())),
    }
}

/// Parses one line of a script. A script drives the running host, so `script` and `help` are
/// refused here, when the file is read, and not after earlier lines have already run.
pub fn parse_script_line(words: &[String]) -> Result<Command, CommandError> {
    match parse(words)? {
        Command::Script(_) => Err(CommandError::NotAllowedInScript("script")),
        Command::Help => Err(CommandError::NotAllowedInScript("help")),
        Command::Repl => Err(CommandError::NotAllowedInScript("repl")),
        command => Ok(command),
    }
}
