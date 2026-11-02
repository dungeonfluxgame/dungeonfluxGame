//! The text of the developer command line, parsed into typed commands.
//!
//! It owns the grammar and the usage text, and nothing else: no sockets, no files, no printing.
//! Values stay text here because only the running canvas knows each field's type, so a spawn or a
//! set is sent with `ValueForm::Text` and the canvas reads each value by its field's type.

use std::collections::BTreeSet;
use std::fmt;
use std::path::PathBuf;

use df_playground::controls::{ControlsChange, Device, MAXIMUM_TRANSITION_MS, TransitionStyle};
use df_playground::copy::ReadingPace;
use df_playground::fixture::{Choice, PartyMember, UnavailableChoice};
use df_playground::scene::{ImageSlot, MAXIMUM_VOLUME, SceneChange, Target};

use crate::wire::{EffectRequest, ValueForm};

const SPAWN_USAGE: &str = "spawn <kind> <id> [x=N] [y=N] [field=value ...]";
const SET_USAGE: &str = "set <id> <field>=<value> [<field>=<value> ...]";
const MOVE_USAGE: &str = "move <id> <x> <y>";
const TRAVEL_USAGE: &str = "travel <id> <x> <y> [speed=N]";
const STOP_USAGE: &str = "stop <id> [<id> ...]";
/// Canvas units per second when `travel` is given no speed: the canvas in about five seconds.
pub const DEFAULT_TRAVEL_SPEED: f32 = 20.0;
const DESPAWN_USAGE: &str = "despawn <id> [<id> ...]";
const EVENTS_USAGE: &str = "events [after]";
const SCRIPT_USAGE: &str = "script <file>";
const PLAYGROUND_USAGE: &str = "playground [set <key>=<value> ... | go <fixture> [transition=cut|fade|wipe] [ms=N] | cycle <a> <b> [times=N] [hold=MS] [transition=..] [ms=N]]";
const WAIT_USAGE: &str = "wait <milliseconds>";
/// Longest `wait` and longest hold in a `cycle`, in milliseconds.
pub const MAXIMUM_WAIT_MS: u64 = 600_000;
/// Most rounds a `cycle` runs.
pub const MAXIMUM_CYCLES: u32 = 100;
/// Rounds of a `cycle` and the pause between scenes when none are given.
pub const DEFAULT_CYCLES: u32 = 2;
pub const DEFAULT_HOLD_MS: u64 = 2_000;
const SCENE_USAGE: &str = "scene caption <text> [speaker=NAME] | caption-clear | party ... | choices ... | image ... | music ... | sound ... | reset";
const CAPTION_USAGE: &str = "scene caption <text> [speaker=NAME]";
const PARTY_USAGE: &str = "scene party add <name> hp=N [max=N] [color=#rrggbb] [conditions=a,b] | set <name> [hp=N] [max=N] [conditions=a,b] | remove <name> | clear";
const IMAGE_USAGE: &str =
    "scene image <tv-backdrop|tv-card|phone> <name> [caption=TEXT] | scene image-clear <slot>";
const MUSIC_USAGE: &str =
    "scene music <name> [on=both|tv|phone] [volume=0-100] [loop=true|false] | scene music-stop";
const SOUND_USAGE: &str = "scene sound <name> [on=both|tv|phone] [volume=0-100]";
const MEDIA_USAGE: &str = "media | media add <file> [as=NAME] | media remove <name>";
/// Volume of music and sounds when none is given, in percent.
pub const DEFAULT_VOLUME: u8 = 80;
const CHOICES_USAGE: &str = "scene choices suggested [<label>[=<explanation>] ...] | group <name> [<label> ...] | unavailable [<label>=<reason> ...] | clear";
const FIXTURE_USAGE: &str =
    "fixture show <name> | fixture write <name> <file> | fixture remove <name>";
/// The keys `playground set` accepts and what each takes.
const PLAYGROUND_KEYS: &str = "fixture=<name>, device=both|tv|phone, pace=slow|normal|fast, status=fixture|<status>, unavailable=true|false, grid=true|false, transition=cut|fade|wipe, ms=0-5000";

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
    /// Read commands from standard input until it ends, one per line, keeping the connection
    /// to the host. Only the command line starts it.
    Repl,
    /// Print what the playground is showing and the fixtures it can show.
    Playground,
    /// Change what the playground shows, in order.
    PlaygroundSet(Vec<ControlsChange>),
    /// Go to `first`, hold, go to `second`, hold, and so on for `times` rounds. The pauses are
    /// kept by the command line, not the host.
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
    /// What each open page says it is showing and playing.
    Clients,
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
         \n  {MOVE_USAGE}\n      move an element at once (x 0 to 100, y 0 to 56.25)\n\
         \n  {TRAVEL_USAGE}\n      send an element toward a point; the host moves it a little every tick, so it\n      glides there on its own (speed in canvas units per second, default 20)\n\
         \n  {STOP_USAGE}\n      end travel where the element stands\n\
         \n  repl\n      read commands from standard input, one per line, until quit; a command costs\n      about a millisecond instead of a process start\n\
         \n  {DESPAWN_USAGE}\n      remove elements\n\
         \n  clear\n      remove every element\n\
         \n  canvas\n      print the canvas as JSON\n\
         \n  kinds\n      list the kinds and their fields\n\
         \n  {EVENTS_USAGE}\n      print the newest events, or those after a sequence\n\
         \n  reload\n      re-read the kind files without a restart\n\
         \n  {PLAYGROUND_USAGE}\n      show or change what the playground page shows; keys: {PLAYGROUND_KEYS}\n\
         \n  {SCENE_USAGE}\n      change the playground's caption, party strip or phone choices live;\n      {PARTY_USAGE}\n      {CHOICES_USAGE}\n      a fixture is not changed; scene reset or another fixture goes back to the file\n\
         \n  {IMAGE_USAGE}\n      put a picture from the media library on the shared screen (tv-backdrop behind the\n      figures, tv-card in the middle) or at the top of a phone's story\n\
         \n  {MUSIC_USAGE}\n      play music until it is stopped or replaced\n\
         \n  {SOUND_USAGE}\n      play a sound once\n\
         \n  {MEDIA_USAGE}\n      list, add (a local file, copied into the library) or remove pictures and sounds\n\
         \n  clients\n      what each open page says it is showing and playing, and whether its sound is\n      locked (a browser plays sound only after someone clicks the page)\n\
         \n  {WAIT_USAGE}\n      pause here; stage scenes over time in a script or a session\n\
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

fn target(value: &str) -> Result<Target, CommandError> {
    Target::from_key(value).ok_or_else(|| CommandError::BadChoice {
        key: "on".to_owned(),
        text: value.to_owned(),
        allowed: "both, tv or phone",
    })
}

fn volume(value: &str) -> Result<u8, CommandError> {
    value
        .parse::<u8>()
        .ok()
        .filter(|volume| *volume <= MAXIMUM_VOLUME)
        .ok_or_else(|| CommandError::BadChoice {
            key: "volume".to_owned(),
            text: value.to_owned(),
            allowed: "a whole number from 0 to 100",
        })
}

fn image(words: &[String]) -> Result<Command, CommandError> {
    let [slot, media, rest @ ..] = words else {
        return Err(CommandError::Usage(IMAGE_USAGE));
    };
    let slot = ImageSlot::from_key(slot).ok_or_else(|| CommandError::BadChoice {
        key: "the picture slot".to_owned(),
        text: slot.clone(),
        allowed: "tv-backdrop, tv-card or phone",
    })?;
    let mut caption = None;
    for (_, value) in keyed(rest, "caption=TEXT", &["caption"])? {
        caption = Some(value.to_owned());
    }
    Ok(Command::Scene(SceneChange::Image {
        slot,
        media: media.clone(),
        caption,
    }))
}

fn music(words: &[String]) -> Result<Command, CommandError> {
    let [media, rest @ ..] = words else {
        return Err(CommandError::Usage(MUSIC_USAGE));
    };
    let (mut on, mut level, mut looping) = (Target::Both, DEFAULT_VOLUME, true);
    for (key, value) in keyed(rest, "on=, volume=, loop=", &["on", "volume", "loop"])? {
        match key {
            "on" => on = target(value)?,
            "volume" => level = volume(value)?,
            _ => looping = truth(key, value)?,
        }
    }
    Ok(Command::Scene(SceneChange::Music {
        media: media.clone(),
        target: on,
        volume: level,
        looping,
    }))
}

fn sound(words: &[String]) -> Result<Command, CommandError> {
    let [media, rest @ ..] = words else {
        return Err(CommandError::Usage(SOUND_USAGE));
    };
    let (mut on, mut level) = (Target::Both, DEFAULT_VOLUME);
    for (key, value) in keyed(rest, "on=, volume=", &["on", "volume"])? {
        match key {
            "on" => on = target(value)?,
            _ => level = volume(value)?,
        }
    }
    Ok(Command::Scene(SceneChange::Sound {
        media: media.clone(),
        target: on,
        volume: level,
    }))
}

fn media(words: &[String]) -> Result<Command, CommandError> {
    match words {
        [] => Ok(Command::Media),
        [verb, name] if verb == "remove" => Ok(Command::MediaRemove(name.clone())),
        [verb, path, rest @ ..] if verb == "add" => {
            let mut name = None;
            for (_, value) in keyed(rest, "as=NAME", &["as"])? {
                name = Some(value.to_owned());
            }
            Ok(Command::MediaAdd {
                path: PathBuf::from(path),
                name,
            })
        }
        _ => Err(CommandError::Usage(MEDIA_USAGE)),
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
        Some((verb, rest)) if verb == "image" => image(rest),
        Some((verb, rest)) if verb == "image-clear" => match rest {
            [slot] => ImageSlot::from_key(slot)
                .map(|slot| Command::Scene(SceneChange::ClearImage { slot }))
                .ok_or_else(|| CommandError::BadChoice {
                    key: "the picture slot".to_owned(),
                    text: slot.clone(),
                    allowed: "tv-backdrop, tv-card or phone",
                }),
            _ => Err(CommandError::Usage(IMAGE_USAGE)),
        },
        Some((verb, rest)) if verb == "music" => music(rest),
        Some((verb, rest)) if verb == "music-stop" && rest.is_empty() => {
            Ok(Command::Scene(SceneChange::StopMusic {}))
        }
        Some((verb, rest)) if verb == "sound" => sound(rest),
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
        "transition" => TransitionStyle::from_key(value)
            .map(ControlsChange::TransitionStyle)
            .ok_or_else(|| CommandError::BadChoice {
                key: key.to_owned(),
                text: value.to_owned(),
                allowed: "cut, fade or wipe",
            }),
        "ms" => value
            .parse::<u32>()
            .map_err(|_| CommandError::BadCount {
                what: "ms",
                text: value.to_owned(),
            })
            .and_then(|ms| {
                if ms > MAXIMUM_TRANSITION_MS {
                    Err(CommandError::BadChoice {
                        key: key.to_owned(),
                        text: value.to_owned(),
                        allowed: "a whole number of milliseconds, at most 5000",
                    })
                } else {
                    Ok(ControlsChange::TransitionMs(ms))
                }
            }),
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
        [word, name, pairs @ ..] if word == "go" => {
            let mut changes = transition_changes(pairs, &["transition", "ms"])?;
            changes.push(ControlsChange::Fixture(name.clone()));
            Ok(Command::PlaygroundSet(changes))
        }
        [word, first, second, pairs @ ..] if word == "cycle" => {
            let mut times = DEFAULT_CYCLES;
            let mut hold_ms = DEFAULT_HOLD_MS;
            let mut settings = Vec::new();
            for text in pairs {
                let (key, value) = pair(text)?;
                match key {
                    "times" => {
                        times = value
                            .parse::<u32>()
                            .ok()
                            .filter(|count| (1..=MAXIMUM_CYCLES).contains(count))
                            .ok_or_else(|| CommandError::BadChoice {
                                key: "times".to_owned(),
                                text: value.to_owned(),
                                allowed: "a whole number from 1 to 100",
                            })?;
                    }
                    "hold" => hold_ms = wait_milliseconds("hold", value)?,
                    _ => settings.push(text.clone()),
                }
            }
            Ok(Command::PlaygroundCycle {
                first: first.clone(),
                second: second.clone(),
                times,
                hold_ms,
                transition: transition_changes(&settings, &["transition", "ms"])?,
            })
        }
        _ => Err(CommandError::Usage(PLAYGROUND_USAGE)),
    }
}

/// Reads `transition=` and `ms=` pairs into changes, in that order, refusing any other key.
fn transition_changes<S: AsRef<str>>(
    pairs: &[S],
    allowed: &[&str],
) -> Result<Vec<ControlsChange>, CommandError> {
    let mut seen = BTreeSet::new();
    let (mut style, mut length) = (None, None);
    for text in pairs {
        let (key, value) = pair(text.as_ref())?;
        if !allowed.contains(&key) {
            return Err(CommandError::BadChoice {
                key: "this command".to_owned(),
                text: key.to_owned(),
                allowed: "transition=cut|fade|wipe, ms=N",
            });
        }
        if !seen.insert(key) {
            return Err(CommandError::DuplicateKey(key.to_owned()));
        }
        match playground_change(key, value)? {
            change @ ControlsChange::TransitionStyle(_) => style = Some(change),
            change => length = Some(change),
        }
    }
    Ok(style.into_iter().chain(length).collect())
}

fn wait_milliseconds(what: &'static str, text: &str) -> Result<u64, CommandError> {
    text.parse::<u64>()
        .ok()
        .filter(|ms| *ms <= MAXIMUM_WAIT_MS)
        .ok_or_else(|| CommandError::BadChoice {
            key: what.to_owned(),
            text: text.to_owned(),
            allowed: "a whole number of milliseconds, at most 600000",
        })
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
        "travel" => travel(rest),
        "stop" => {
            if rest.is_empty() {
                return Err(CommandError::Usage(STOP_USAGE));
            }
            Ok(Command::Effects(
                rest.iter()
                    .map(|id| EffectRequest::Stop { id: id.clone() })
                    .collect(),
            ))
        }
        "repl" if rest.is_empty() => Ok(Command::Repl),
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
        "wait" => match rest {
            [text] => wait_milliseconds("wait", text).map(Command::Wait),
            _ => Err(CommandError::Usage(WAIT_USAGE)),
        },
        "media" => media(rest),
        "clients" if rest.is_empty() => Ok(Command::Clients),
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
        Command::Repl => Err(CommandError::NotAllowedInScript("repl")),
        command => Ok(command),
    }
}
