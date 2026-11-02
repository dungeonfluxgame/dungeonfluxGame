//! Everything that crosses the developer API, in one place: the routes, the request a client
//! sends and the answers the server gives.
//!
//! The server reads and writes these types and the command line writes and reads them, so a
//! rename breaks the build instead of a running command. It owns the shape and nothing else: it
//! never touches the canvas, a socket or a file.

pub use df_playground::controls::ControlsReply;
use df_table::event::RejectionCause;
use serde::{Deserialize, Serialize};

use crate::event_log::LoggedLine;

/// The addresses of the API. `CANVAS` and `EVENTS` answer GET; `EFFECT` and `RELOAD` answer POST
/// with JSON.
pub mod route {
    pub const CANVAS: &str = "/canvas.json";
    pub const EVENTS: &str = "/events.json";
    pub const EFFECT: &str = "/dev/effect";
    pub const RELOAD: &str = "/dev/reload";

    /// The playground page, its theme, its fixtures and its compiled package (all GET).
    pub const PLAYGROUND: &str = "/playground";
    pub const PLAYGROUND_SLASH: &str = "/playground/";
    pub const PLAYGROUND_THEME: &str = "/playground/theme.css";
    /// Changes when any playground file does: `<assets>.<package>`.
    pub const PLAYGROUND_REVISION: &str = "/playground/revision";
    pub const PLAYGROUND_FIXTURE_LIST: &str = "/playground/fixtures.json";
    pub const PLAYGROUND_FIXTURE: &str = "/playground/fixtures/{name}";
    pub const PLAYGROUND_PACKAGE_FILE: &str = "/playground/pkg/{name}";
    /// What the playground page is showing (GET), so the page and the dev CLI agree.
    pub const PLAYGROUND_CONTROLS: &str = "/playground/controls.json";
    /// Changes what the playground shows (POST a `ControlsChange`).
    pub const DEV_PLAYGROUND_CONTROLS: &str = "/dev/playground/controls";
    /// Changes the live scene: caption, party, choices (POST a `SceneChange`).
    pub const DEV_PLAYGROUND_SCENE: &str = "/dev/playground/scene";
    /// Writes a fixture file (POST a `FixtureWrite`).
    pub const DEV_FIXTURE_WRITE: &str = "/dev/playground/fixture";
    /// Deletes a fixture file (POST a `FixtureRemove`).
    pub const DEV_FIXTURE_REMOVE: &str = "/dev/playground/fixture/remove";
}

/// The query parameter of `route::EVENTS` that names the last sequence already seen.
pub const EVENTS_AFTER_PARAMETER: &str = "after";
/// Most notices or problems one answer carries; more are summarized as "and N more".
pub const MAXIMUM_LISTED_PROBLEMS: usize = 50;

/// How the values in a request are written.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValueForm {
    /// Each value is JSON of the field's type: a number is a number.
    #[default]
    Json,
    /// Each value is text, as typed on a command line, and the field's type reads it.
    Text,
}

/// One change to the canvas, as sent to the API.
///
/// `Clear {}` is a struct variant on purpose: serde ignores unknown keys on a unit variant of a
/// tagged enum, so only an empty struct refuses them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum EffectRequest {
    Spawn {
        id: String,
        kind: String,
        /// Where the element goes; the middle of the canvas when left out.
        x: Option<f32>,
        y: Option<f32>,
        #[serde(default)]
        fields: serde_json::Map<String, serde_json::Value>,
        #[serde(default)]
        values: ValueForm,
    },
    Despawn {
        id: String,
    },
    Set {
        id: String,
        field: String,
        value: serde_json::Value,
        #[serde(default)]
        values: ValueForm,
    },
    Move {
        id: String,
        x: f32,
        y: f32,
    },
    Clear {},
}

/// The answer to a request that changes something.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum Reply {
    /// The change was made. `notices` carries things worth knowing, such as elements that no
    /// longer fit a reloaded kind.
    Done { revision: u64, notices: Vec<String> },
    /// The change was not made. `cause` is absent for a rejected reload, which has no single
    /// cause; `problems` lists what is wrong.
    Rejected {
        cause: Option<RejectionCause>,
        message: String,
        problems: Vec<String>,
    },
}

/// A fixture to write: `name` with or without `.json`, and the whole file's text. The text must
/// read as a fixture, or nothing is written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FixtureWrite {
    pub name: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FixtureRemove {
    pub name: String,
}

/// The answer to a request for events.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventsReply {
    /// Oldest first.
    pub events: Vec<LoggedLine>,
    /// The newest sequence logged, so a reader can tell the host restarted when it falls below the
    /// last sequence the reader saw.
    pub latest: u64,
}

/// Keeps at most `MAXIMUM_LISTED_PROBLEMS` entries, replacing the rest with a count.
pub fn capped(mut problems: Vec<String>) -> Vec<String> {
    if problems.len() > MAXIMUM_LISTED_PROBLEMS {
        let more = problems.len() - MAXIMUM_LISTED_PROBLEMS;
        problems.truncate(MAXIMUM_LISTED_PROBLEMS);
        problems.push(format!("and {more} more"));
    }
    problems
}
