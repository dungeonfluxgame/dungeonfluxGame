//! Everything that crosses the developer API, in one place: the routes, the request a client
//! sends and the answers the server gives.
//!
//! The server reads and writes these types and the command line writes and reads them, so a
//! rename breaks the build instead of a running command. It owns the shape and nothing else: it
//! never touches the canvas, a socket or a file.

pub use df_playground::controls::ControlsReply;
use df_playground::media::ClientReport;
use df_table::event::RejectionCause;
use serde::{Deserialize, Serialize};

use crate::event_log::LoggedLine;
use crate::limits::MAXIMUM_LISTED_PROBLEMS;
use crate::view::FrameCanvas;

/// The addresses of the API. `CANVAS` and `EVENTS` answer GET; `EFFECT` and `RELOAD` answer POST
/// with JSON. The addresses the playground page asks for are `df_playground::path`'s, so the page
/// and the router cannot drift apart.
pub mod path {
    pub const CANVAS: &str = df_playground::path::CANVAS;
    pub const EVENTS: &str = "/events.json";
    pub const EFFECT: &str = "/dev/effect";
    pub const RELOAD: &str = "/dev/reload";

    /// The playground page, its theme, its fixtures and its compiled package (all GET).
    pub const PLAYGROUND: &str = "/playground";
    pub const PLAYGROUND_SLASH: &str = "/playground/";
    pub const PLAYGROUND_THEME: &str = df_playground::path::THEME;
    /// Changes when any playground file does: `<assets>.<package>`.
    pub const PLAYGROUND_REVISION: &str = df_playground::path::REVISION;
    pub const PLAYGROUND_FIXTURE_LIST: &str = df_playground::path::FIXTURE_LIST;
    pub const PLAYGROUND_FIXTURE: &str = df_playground::path::FIXTURE_FILE;
    pub const PLAYGROUND_PACKAGE_FILE: &str = "/playground/pkg/{name}";
    /// A long poll for the next frame (GET with `?after=N&boot=B`): answers at once when the host
    /// has changed since frame N or is not the run B, otherwise holds the request until it changes
    /// or a wait runs out.
    pub const PLAYGROUND_FRAME: &str = df_playground::path::FRAME;
    /// The media library: the list and each file (GET).
    pub const PLAYGROUND_MEDIA_LIST: &str = "/playground/media.json";
    pub const PLAYGROUND_MEDIA_FILE: &str = df_playground::path::MEDIA_FILE;
    /// What each open page says it is showing and playing (GET).
    pub const PLAYGROUND_CLIENTS: &str = "/playground/clients.json";
    /// A page tells the host what it is showing and playing (POST a `ClientReportPost`). It does
    /// not wake other pages.
    pub const DEV_CLIENT_REPORT: &str = df_playground::path::CLIENT_REPORT;
    /// Adds or replaces a media file (POST a `MediaAdd`).
    pub const DEV_MEDIA_ADD: &str = "/dev/playground/media";
    /// Deletes a media file (POST a `MediaRemove`).
    pub const DEV_MEDIA_REMOVE: &str = "/dev/playground/media/remove";
    /// What the playground page is showing (GET), so the page and the dev CLI agree.
    pub const PLAYGROUND_CONTROLS: &str = df_playground::path::CONTROLS;
    /// Changes what the playground shows (POST a `ControlsChange`).
    pub const DEV_PLAYGROUND_CONTROLS: &str = df_playground::path::CONTROLS_CHANGE;
    /// Several changes of what the playground shows, applied together or not at all (POST a
    /// `ControlsBatch`).
    pub const DEV_PLAYGROUND_CONTROLS_BATCH: &str = "/dev/playground/controls/batch";
    /// Changes the live scene: caption, party, choices (POST a `SceneChange`).
    pub const DEV_PLAYGROUND_SCENE: &str = "/dev/playground/scene";
    /// Writes a fixture file (POST a `FixtureWrite`).
    pub const DEV_FIXTURE_WRITE: &str = "/dev/playground/fixture";
    /// Deletes a fixture file (POST a `FixtureRemove`).
    pub const DEV_FIXTURE_REMOVE: &str = "/dev/playground/fixture/remove";
}

/// Transitional name of [`path`], kept while `server.rs`, `dev.rs`, `engine/cli_play.rs` and two
/// route test files still say `route::`; the last of them removes it.
pub use self::path as route;

/// The query parameter of `path::PLAYGROUND_FRAME` that names the last frame already seen.
pub const FRAME_AFTER_PARAMETER: &str = df_playground::path::FRAME_AFTER_PARAMETER;
/// The query parameter of `path::PLAYGROUND_FRAME` that names the run of the host the last frame
/// came from: a page that names another run is answered at once.
pub const FRAME_BOOT_PARAMETER: &str = df_playground::path::FRAME_BOOT_PARAMETER;
/// The query parameter of `path::PLAYGROUND_FRAME` with which a page says it accepts an empty
/// answer when nothing changed before the wait ran out: `idle=204`. Without it the host answers
/// the timeout with the whole frame, as it always did.
pub const FRAME_IDLE_PARAMETER: &str = df_playground::path::FRAME_IDLE_PARAMETER;
/// The value of `FRAME_IDLE_PARAMETER` that asks for a bodiless 204 on a timeout.
pub const FRAME_IDLE_NO_CONTENT: u16 = df_playground::path::FRAME_IDLE_NO_CONTENT;
/// The query parameter of `path::EVENTS` that names the last sequence already seen.
pub const EVENTS_AFTER_PARAMETER: &str = "after";

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
    /// Sends the element toward a point at a speed in canvas units per second; the host's clock
    /// moves it a little each tick.
    Travel {
        id: String,
        x: f32,
        y: f32,
        speed: f32,
    },
    /// Ends the element's travel where it stands.
    Stop {
        id: String,
    },
    /// Turns the element clockwise to `rotation_degrees` (0 to 359) about `pivot`, or about the
    /// pivot it already has when none is given. The numbers are any JSON number on purpose, so a
    /// value out of range, fractional or larger than 64 bits is refused with its range instead of
    /// failing to read.
    Transform {
        id: String,
        rotation_degrees: serde_json::Number,
        #[serde(default)]
        pivot: Option<PivotRequest>,
    },
    Clear {},
}

/// The point an element turns about, as whole percent of its own width and height (0 to 100;
/// 50 and 50 is the centre).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PivotRequest {
    pub x_percent: serde_json::Number,
    pub y_percent: serde_json::Number,
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

/// Changes of what the playground shows that are applied together: if one is refused none is
/// applied, and the whole is one numbered change with one audit line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlsBatch {
    pub changes: Vec<df_playground::controls::ControlsChange>,
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

/// A media file to add or replace: a plain name ending in a known type, and the file's bytes as
/// base64.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MediaAdd {
    pub name: String,
    pub data_base64: String,
}

impl MediaAdd {
    /// The text around the base64 data of an add sent in pieces: the opening, which carries the
    /// quoted name, and the closing. A streaming sender writes the opening, the encoded bytes and
    /// the closing, so a file is never held whole.
    pub fn streamed_ends(name: &str) -> Result<(String, &'static str), serde_json::Error> {
        let quoted_name = serde_json::to_string(name)?;
        Ok((
            format!("{{\"name\":{quoted_name},\"data_base64\":\""),
            "\"}",
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MediaRemove {
    pub name: String,
}

/// One open page and what it last said, with how long ago.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientView {
    pub client: String,
    pub age_ms: u64,
    pub report: ClientReport,
}

/// Everything the playground page draws, taken at one moment: the canvas and what the
/// playground is showing. `frame` counts the host's changes, so a page that has frame N asks for
/// the next one with `after=N` and is answered when there is one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Frame {
    pub frame: u64,
    pub canvas: FrameCanvas,
    pub controls: ControlsReply,
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
