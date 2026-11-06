//! The commands that read the host itself: `events`, `host status`, `frame` and `clients`.
//!
//! `events` and `clients timeline` carry typed queries (an area is checked as the host checks it,
//! a kind is a `TimelineKind`), so a typo is refused here and never reaches a route.

use df_playground::timeline::TimelineKind;

use super::usage::{CLIENTS_USAGE, EVENTS_USAGE, HOST_USAGE};
use super::{Command, CommandError, bad_choice, keyed, split_pair_words};
use crate::observe::EventLevel;
use crate::playground::check_client_name;
use crate::server::request::LineArea;

/// The query of a plain `events` read: lines after a sequence, of one area, at most a number.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EventsRead {
    pub after: Option<u64>,
    pub area: Option<LineArea>,
    pub limit: Option<usize>,
}

/// The query of a `clients timeline` read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimelineRead {
    pub client: String,
    pub kind: Option<TimelineKind>,
    pub after: Option<u64>,
}

const LEVEL_CHOICES: &str = "always or verbose";
const KIND_CHOICES: &str = "line, plate, figure, clip, scene, sound, music, draw, camera or error";

fn events_after(text: &str) -> Result<u64, CommandError> {
    text.parse::<u64>().map_err(|_| CommandError::BadNumber {
        what: "the events sequence",
        text: text.to_owned(),
    })
}

fn events_limit(text: &str) -> Result<usize, CommandError> {
    text.parse::<usize>()
        .ok()
        .filter(|limit| *limit > 0)
        .ok_or_else(|| bad_choice("limit", text, "a whole number from 1"))
}

fn events_area(text: &str) -> Result<LineArea, CommandError> {
    LineArea::new(text).map_err(|_| {
        bad_choice(
            "area",
            text,
            "the first word of a line: uppercase letters, digits and '_'",
        )
    })
}

/// Reads the words after `events`: a read, or `level [always|verbose]`.
pub(super) fn events(words: &[String]) -> Result<Command, CommandError> {
    if let [word, rest @ ..] = words
        && word == "level"
    {
        return match rest {
            [] => Ok(Command::EventsLevel(None)),
            [level] => EventLevel::from_key(level)
                .map(|level| Command::EventsLevel(Some(level)))
                .ok_or_else(|| bad_choice("level", level, LEVEL_CHOICES)),
            _ => Err(CommandError::Usage(EVENTS_USAGE)),
        };
    }
    let (positional, pairs) = split_pair_words(words);
    let mut read = EventsRead::default();
    match positional.as_slice() {
        [] => {}
        [after] => read.after = Some(events_after(after)?),
        _ => return Err(CommandError::Usage(EVENTS_USAGE)),
    }
    for (key, value) in keyed(&pairs, "area=AREA, limit=N", &["area", "limit"])? {
        match key {
            "area" => read.area = Some(events_area(value)?),
            _ => read.limit = Some(events_limit(value)?),
        }
    }
    Ok(Command::Events(read))
}

/// Reads the words after `host`: `status`.
pub(super) fn host(words: &[String]) -> Result<Command, CommandError> {
    match words {
        [word] if word == "status" => Ok(Command::HostStatus),
        _ => Err(CommandError::Usage(HOST_USAGE)),
    }
}

/// Reads the words after `frame`: an optional `after=N`.
pub(super) fn frame(words: &[String]) -> Result<Command, CommandError> {
    let mut after = None;
    for (_, value) in keyed(words, "after=N", &["after"])? {
        after = Some(value.parse::<u64>().map_err(|_| CommandError::BadNumber {
            what: "the frame count",
            text: value.to_owned(),
        })?);
    }
    Ok(Command::Frame { after })
}

fn client_name(text: &str) -> Result<String, CommandError> {
    check_client_name(text)
        .map(|()| text.to_owned())
        .map_err(|error| bad_choice("client", text, error.to_string()))
}

/// Reads the words after `clients`: nothing, a page's name, or `timeline <client> ...`.
pub(super) fn clients(words: &[String]) -> Result<Command, CommandError> {
    match words {
        [] => Ok(Command::Clients),
        [word, client, rest @ ..] if word == "timeline" => {
            let mut read = TimelineRead {
                client: client_name(client)?,
                kind: None,
                after: None,
            };
            for (key, value) in keyed(rest, "kind=KIND, after=SEQ", &["kind", "after"])? {
                match key {
                    "kind" => {
                        read.kind = Some(
                            TimelineKind::from_key(value)
                                .ok_or_else(|| bad_choice("kind", value, KIND_CHOICES))?,
                        );
                    }
                    _ => {
                        read.after =
                            Some(value.parse::<u64>().map_err(|_| CommandError::BadNumber {
                                what: "the timeline sequence",
                                text: value.to_owned(),
                            })?);
                    }
                }
            }
            Ok(Command::ClientsTimeline(read))
        }
        [word] if word == "timeline" => Err(CommandError::Usage(CLIENTS_USAGE)),
        [client] => client_name(client).map(Command::ClientShow),
        _ => Err(CommandError::Usage(CLIENTS_USAGE)),
    }
}
