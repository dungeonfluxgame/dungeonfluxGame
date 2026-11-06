//! What the person (or the dev CLI) has chosen for the playground: which scenario, which devices,
//! the reading pace, a status to try, and two display toggles.
//!
//! It is plain data plus the one rule for changing it, shared by the page and the host so both
//! agree on what a change means. It never draws and never reads the page.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::copy::{ReadingPace, status_from_key};
use crate::fixture::Status;
use crate::scene::SceneOverlay;

use crate::limits::{DEFAULT_TRANSITION_MS, MAXIMUM_TRANSITION_MS};

/// The key of the menu entry that keeps the fixture's own status.
pub const FIXTURE_STATUS_KEY: &str = "fixture";

/// Which devices are drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Device {
    Both,
    Tv,
    Phone,
}

impl Device {
    pub const ALL: [Self; 3] = [Self::Both, Self::Tv, Self::Phone];

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|value| value.key() == key)
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Both => "both",
            Self::Tv => "tv",
            Self::Phone => "phone",
        }
    }

    pub fn shows_tv(self) -> bool {
        matches!(self, Self::Both | Self::Tv)
    }

    pub fn shows_phone(self) -> bool {
        matches!(self, Self::Both | Self::Phone)
    }
}

/// How the shared screen and the phone leave one scene for the next.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransitionStyle {
    /// The new scene replaces the old at once.
    Cut,
    /// The old scene fades away over the new one.
    Fade,
    /// The old scene is wiped off to the left, uncovering the new one.
    Wipe,
}

impl TransitionStyle {
    pub const ALL: [Self; 3] = [Self::Cut, Self::Fade, Self::Wipe];

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|value| value.key() == key)
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Cut => "cut",
            Self::Fade => "fade",
            Self::Wipe => "wipe",
        }
    }
}

/// What bounds the shared screen's camera zoom.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ZoomLimit {
    /// Only the model's largest zoom applies, so the camera may draw a plate larger than its own
    /// pixels.
    #[default]
    Model,
    /// The camera never draws the plate larger than its own pixels.
    Pixels,
}

impl ZoomLimit {
    pub const ALL: [Self; 2] = [Self::Model, Self::Pixels];

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|value| value.key() == key)
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Model => "model",
            Self::Pixels => "pixels",
        }
    }
}

/// The style and length of the transition used the next time the scene changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Transition {
    pub style: TransitionStyle,
    pub duration_ms: u64,
}

impl Default for Transition {
    fn default() -> Self {
        Self {
            style: TransitionStyle::Fade,
            duration_ms: DEFAULT_TRANSITION_MS,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Controls {
    /// The fixture file shown, or none while no fixture exists.
    pub fixture: Option<String>,
    pub device: Device,
    pub pace: ReadingPace,
    /// A status key from the menu, or `FIXTURE_STATUS_KEY` to use the fixture's status.
    pub status_key: String,
    pub show_unavailable: bool,
    pub show_grid: bool,
    /// Whether the backdrop's scanned layout (SAM regions and head points) is drawn over it.
    #[serde(default = "shown_by_default")]
    pub show_sam: bool,
    /// How the next change of scene is drawn.
    #[serde(default)]
    pub transition: Transition,
    /// What bounds the camera's zoom.
    #[serde(default)]
    pub zoom_limit: ZoomLimit,
}

fn shown_by_default() -> bool {
    true
}

impl Default for Controls {
    fn default() -> Self {
        Self {
            fixture: None,
            device: Device::Both,
            pace: ReadingPace::Normal,
            status_key: FIXTURE_STATUS_KEY.to_owned(),
            show_unavailable: false,
            show_grid: false,
            show_sam: true,
            transition: Transition::default(),
            zoom_limit: ZoomLimit::default(),
        }
    }
}

/// The controls, the scene changes and how many changes there have been, as the host answers: a
/// reader that has the same revision has nothing new to draw.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ControlsReply {
    pub revision: u64,
    /// Names this run of the host. Every counter (revision, cue, slide and camera serials) starts
    /// again when the host does, so a page keys what it remembers by this too and drops it all
    /// when it changes. Empty from a host that does not say.
    #[serde(default)]
    pub boot_id: String,
    pub controls: Controls,
    /// Live changes to the scene, drawn on top of the fixture.
    #[serde(default)]
    pub scene: SceneOverlay,
}

impl ControlsReply {
    /// The reply as a reader should see it `now_ms` after the host started: the moves in the
    /// scene say how long ago they began.
    pub fn at(&self, now_ms: u64) -> Self {
        Self {
            scene: self.scene.at(now_ms),
            ..self.clone()
        }
    }
}

/// The host's answer to a change of what the playground shows (`path::CONTROLS_CHANGE`), as far as
/// the page reads it: `{"outcome":"done","value":..}` with the controls after the change, or
/// `{"outcome":"rejected","error":{..},"message":".."}`. The page shows the host's sentence and
/// does not read the typed `error`, which is for the dev CLI.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum ControlsChangeReply {
    /// Boxed: the controls are far larger than a refusal.
    Done {
        value: Box<ControlsReply>,
    },
    Rejected {
        message: String,
    },
}

impl ControlsChangeReply {
    /// The controls after the change, or the host's sentence for its refusal.
    pub fn into_result(self) -> Result<ControlsReply, String> {
        match self {
            Self::Done { value } => Ok(*value),
            Self::Rejected { message } => Err(message),
        }
    }
}

/// One change to the controls, as the page and the dev CLI both send it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "change", content = "value", rename_all = "snake_case")]
pub enum ControlsChange {
    Fixture(String),
    Device(Device),
    Pace(ReadingPace),
    /// A status key (see `copy::STATUS_KEYS`) or `FIXTURE_STATUS_KEY`.
    Status(String),
    ShowUnavailable(bool),
    ShowGrid(bool),
    /// Shows or hides the backdrop's scanned layout.
    ShowSam(bool),
    /// Sets how the next change of scene is drawn; the scene itself is not changed.
    TransitionStyle(TransitionStyle),
    /// Sets how long the next change of scene takes, up to `MAXIMUM_TRANSITION_MS`.
    TransitionMs(u64),
    /// Sets what bounds the camera's zoom.
    ZoomLimit(ZoomLimit),
}

/// Why a change was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlsError {
    UnknownStatus(String),
    EmptyFixtureName,
    TransitionTooLong(u64),
}

impl fmt::Display for ControlsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownStatus(key) => write!(
                formatter,
                "'{key}' is not a status; use '{FIXTURE_STATUS_KEY}' or one of {}",
                crate::copy::STATUS_KEYS.join(", ")
            ),
            Self::EmptyFixtureName => formatter.write_str("a fixture name cannot be empty"),
            Self::TransitionTooLong(ms) => write!(
                formatter,
                "a transition is at most {MAXIMUM_TRANSITION_MS} ms, not {ms}"
            ),
        }
    }
}

impl std::error::Error for ControlsError {}

impl Controls {
    /// The status to draw: the chosen one, or the fixture's own when none is chosen.
    pub fn status_for(&self, fixture_status: &Status) -> Status {
        status_from_key(&self.status_key).unwrap_or_else(|| fixture_status.clone())
    }

    /// Makes one change. A refused change leaves the controls as they were.
    pub fn apply(&mut self, change: ControlsChange) -> Result<(), ControlsError> {
        match change {
            ControlsChange::Fixture(name) => {
                if name.is_empty() {
                    return Err(ControlsError::EmptyFixtureName);
                }
                self.fixture = Some(name);
            }
            ControlsChange::Device(device) => self.device = device,
            ControlsChange::Pace(pace) => self.pace = pace,
            ControlsChange::Status(key) => {
                if key != FIXTURE_STATUS_KEY && status_from_key(&key).is_none() {
                    return Err(ControlsError::UnknownStatus(key));
                }
                self.status_key = key;
            }
            ControlsChange::ShowUnavailable(shown) => self.show_unavailable = shown,
            ControlsChange::ShowGrid(shown) => self.show_grid = shown,
            ControlsChange::ShowSam(shown) => self.show_sam = shown,
            ControlsChange::TransitionStyle(style) => self.transition.style = style,
            ControlsChange::TransitionMs(ms) => {
                if ms > MAXIMUM_TRANSITION_MS {
                    return Err(ControlsError::TransitionTooLong(ms));
                }
                self.transition.duration_ms = ms;
            }
            ControlsChange::ZoomLimit(limit) => self.zoom_limit = limit,
        }
        Ok(())
    }
}
