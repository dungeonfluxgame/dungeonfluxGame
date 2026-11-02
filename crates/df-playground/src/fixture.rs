//! The data a playground scene is drawn from, read from a JSON fixture file.
//!
//! It owns the shape of a fixture and its strict reading: a misspelt key is an error with the
//! position of the mistake, so an author sees it on the page instead of a blank panel. It never
//! draws anything and never decides what a status or a choice means: `copy` and `choices` do.

use serde::{Deserialize, Serialize};

use crate::scene::Track;

/// Where the stage gets its elements from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ElementSource {
    /// The elements listed in the fixture.
    Inline(Vec<StageElement>),
    /// The elements on the running canvas, as the `df-table` host serves them.
    Live(LiveMarker),
}

/// The only text accepted where a fixture asks for the live canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LiveMarker {
    Live,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ElementShape {
    Circle,
    Rect,
}

/// One thing drawn on the stage. The host's `/canvas.json` elements have this shape (plus fields
/// the stage ignores), so a live element and an inline one are drawn the same way.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StageElement {
    pub id: String,
    pub kind: String,
    pub label: String,
    /// Centre of the element, in canvas units (x from 0 to 100, y from 0 to 56.25).
    pub x: f32,
    pub y: f32,
    pub color: Option<String>,
    pub shape: ElementShape,
    pub width_units: f32,
    pub height_units: f32,
    /// The element's fields, as the host serves them. A `portrait` field naming a picture in the
    /// media library draws that picture in the figure.
    #[serde(default)]
    pub fields: serde_json::Map<String, serde_json::Value>,
}

impl StageElement {
    /// The media file of the element's portrait, when it has one.
    pub fn portrait(&self) -> Option<&str> {
        self.fields
            .get("portrait")
            .and_then(serde_json::Value::as_str)
            .filter(|name| !name.is_empty())
    }
}

/// What the table is waiting for or showing. Each value has fixed plain-language copy (`copy`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Status {
    Ok,
    Reading,
    Rolling,
    Telling,
    ArtDelayed,
    Degraded,
    ReadOnly,
    Reconnecting,
    WaitingFor { who: String, what: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Caption {
    pub speaker: Option<String>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartyMember {
    pub name: String,
    pub hit_points: u32,
    pub maximum_hit_points: u32,
    #[serde(default)]
    pub conditions: Vec<String>,
    pub color: Option<String>,
}

/// What the shared screen shows.
/// A picture that slides sideways across the shared screen: it starts at `from_percent` of the
/// screen's width and ends at `to_percent`, taking `duration_ms`, at an even pace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Slide {
    pub from_percent: u8,
    pub to_percent: u8,
    pub duration_ms: u32,
    /// Counts the slides of a session, so a page can tell a new slide from one it has already
    /// begun and carry on from where it was after a redraw. The host sets it; leave it out of a
    /// scene file.
    #[serde(default)]
    pub serial: u64,
}

/// Where the shared screen's camera looks: in on one element, or out to the whole stage.
///
/// The camera moves from wherever it was to this framing in `duration_ms`, at an even pace. Only
/// the picture moves: the caption, the party strip and the status stay where they are.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Camera {
    /// The element to keep in the middle of the screen, or `None` for the whole stage.
    pub target: Option<String>,
    /// How far in, in percent: 100 shows the whole stage, 200 shows it twice as large.
    pub zoom_percent: u16,
    /// How long the move takes, in milliseconds. Zero cuts straight to the framing.
    pub duration_ms: u32,
    /// Counts the camera moves of a session, so a page can tell a new move from one it has
    /// already begun and carry on from where it was after a redraw. The host sets it; leave it out
    /// of a scene file.
    #[serde(default)]
    pub serial: u64,
}

/// A picture from the media library, by file name, with an optional line under it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageRef {
    pub media: String,
    pub caption: Option<String>,
    /// Clockwise turn in degrees, 0 to 359 (180 shows it upside down). Defaults to 0.
    #[serde(default)]
    pub rotation_degrees: u16,
    /// A slide across the shared screen, for a card.
    #[serde(default)]
    pub slide: Option<Slide>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TvScene {
    pub place: String,
    pub status: Status,
    pub caption: Option<Caption>,
    /// A picture drawn behind the figures, filling the stage.
    pub backdrop: Option<ImageRef>,
    /// A picture shown in the middle of the screen, such as a handout or a portrait.
    pub card: Option<ImageRef>,
    /// Where the camera looks; without one the whole stage shows.
    #[serde(default)]
    pub camera: Option<Camera>,
    #[serde(default)]
    pub party: Vec<PartyMember>,
    pub elements: ElementSource,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Choice {
    pub label: String,
    pub explain: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChoiceGroup {
    pub name: String,
    pub choices: Vec<Choice>,
}

/// A choice the hero cannot take now, and why.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnavailableChoice {
    pub label: String,
    pub reason: String,
}

/// A private note shown to one seat only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoteCard {
    pub text: String,
}

/// What one player's phone shows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhoneScene {
    pub hero: String,
    pub goal: Option<String>,
    pub status: Status,
    #[serde(default)]
    pub narration: Vec<String>,
    #[serde(default)]
    pub suggested: Vec<Choice>,
    #[serde(default)]
    pub groups: Vec<ChoiceGroup>,
    #[serde(default)]
    pub unavailable: Vec<UnavailableChoice>,
    pub note: Option<NoteCard>,
    /// A picture shown at the top of the story, such as a handout or a portrait.
    pub image: Option<ImageRef>,
}

/// One complete scenario: the shared screen and one phone, drawn together.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fixture {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub tv: TvScene,
    pub phone: PhoneScene,
    /// Music that plays while this scene is shown, from the media library.
    #[serde(default)]
    pub music: Option<Track>,
}

impl Fixture {
    /// Reads a fixture, naming the line and column of the first mistake.
    pub fn parse(text: &str) -> Result<Self, FixtureError> {
        serde_json::from_str(text).map_err(|error| FixtureError {
            line: error.line(),
            column: error.column(),
            message: error.to_string(),
        })
    }
}

/// A fixture that could not be read, with where the reader stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixtureError {
    pub line: usize,
    pub column: usize,
    pub message: String,
}
