//! The commands about what the playground shows: `playground`, `wait`, `media` and `fixture`.
//!
//! `go` and `cycle` are one change each, so the grammar reads them into one list of changes the
//! host applies together or not at all. `set` is a list the command line sends one change at a
//! time, as its help says.

use std::collections::BTreeSet;
use std::path::PathBuf;

use df_playground::controls::{ControlsChange, Device, TransitionStyle};
use df_playground::copy::ReadingPace;
use df_playground::limits::MAXIMUM_TRANSITION_MS;

use super::usage::{FIXTURE_USAGE, MEDIA_USAGE, PLAYGROUND_USAGE, playground_keys};
use super::{Command, CommandError, bad_choice, keyed, pair, truth};
use crate::limits::{MAXIMUM_CYCLES, MAXIMUM_WAIT_MS};

/// Rounds of a `cycle` and the pause between scenes when none are given.
pub const DEFAULT_CYCLES: u32 = 2;
pub const DEFAULT_HOLD_MS: u64 = 2_000;

fn playground_change(key: &str, value: &str) -> Result<ControlsChange, CommandError> {
    match key {
        "fixture" => Ok(ControlsChange::Fixture(value.to_owned())),
        "device" => Device::from_key(value)
            .map(ControlsChange::Device)
            .ok_or_else(|| bad_choice(key, value, "both, tv or phone")),
        "pace" => ReadingPace::from_key(value)
            .map(ControlsChange::Pace)
            .ok_or_else(|| bad_choice(key, value, "slow, normal or fast")),
        "status" => Ok(ControlsChange::Status(value.to_owned())),
        "transition" => TransitionStyle::from_key(value)
            .map(ControlsChange::TransitionStyle)
            .ok_or_else(|| bad_choice(key, value, "cut, fade or wipe")),
        "ms" => value
            .parse::<u64>()
            .map_err(|_| CommandError::BadCount {
                what: "ms",
                text: value.to_owned(),
            })
            .and_then(|ms| {
                if ms > MAXIMUM_TRANSITION_MS {
                    Err(bad_choice(
                        key,
                        value,
                        format!("a whole number of milliseconds, at most {MAXIMUM_TRANSITION_MS}"),
                    ))
                } else {
                    Ok(ControlsChange::TransitionMs(ms))
                }
            }),
        "unavailable" => truth(key, value).map(ControlsChange::ShowUnavailable),
        "grid" => truth(key, value).map(ControlsChange::ShowGrid),
        "sam" => truth(key, value).map(ControlsChange::ShowSam),
        _ => Err(bad_choice("playground set", key, playground_keys())),
    }
}

/// Reads the words after `playground`.
pub(super) fn playground(words: &[String]) -> Result<Command, CommandError> {
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
            Ok(Command::PlaygroundGo(changes))
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
                            .ok_or_else(|| {
                                bad_choice(
                                    "times",
                                    value,
                                    format!("a whole number from 1 to {MAXIMUM_CYCLES}"),
                                )
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
            return Err(bad_choice(
                "this command",
                key,
                "transition=cut|fade|wipe, ms=N",
            ));
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

/// A whole number of milliseconds up to `MAXIMUM_WAIT_MS`, for `wait` and a cycle's `hold`.
pub(super) fn wait_milliseconds(what: &'static str, text: &str) -> Result<u64, CommandError> {
    text.parse::<u64>()
        .ok()
        .filter(|ms| *ms <= MAXIMUM_WAIT_MS)
        .ok_or_else(|| {
            bad_choice(
                what,
                text,
                format!("a whole number of milliseconds, at most {MAXIMUM_WAIT_MS}"),
            )
        })
}

pub(super) fn media(words: &[String]) -> Result<Command, CommandError> {
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
        [verb, rest @ ..] if verb == "generate" => super::media_generate::parse(rest),
        [verb, rest @ ..] if verb == "locate" => crate::locate::cli::parse(rest),
        _ => Err(CommandError::Usage(MEDIA_USAGE)),
    }
}

pub(super) fn fixture(words: &[String]) -> Result<Command, CommandError> {
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
