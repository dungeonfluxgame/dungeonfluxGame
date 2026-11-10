//! What the person (or the dev CLI) has chosen for the playground: which scenario, which devices,
//! the reading pace, a status to try, and two display toggles.
//!
//! It is plain data plus the one rule for changing it, shared by the page and the host so both
//! agree on what a change means. It never draws and never reads the page.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::copy::{ReadingPace, status_from_key};
use crate::dev_reply::HostReply;
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
    /// Whether the backdrop's scanned layout (SAM regions and head points) is drawn over it. Off
    /// by default (owner, 2026-11-08): the plate is shown clean, and a developer turns it on.
    #[serde(default)]
    pub show_sam: bool,
    /// Whether the host voices lines (text to speech). Off, a line is shown and not spoken. Off
    /// by default: voice costs money and needs a key, so a developer turns it on.
    #[serde(default)]
    pub tts_on: bool,
    /// Whether the host takes spoken questions (speech to text). Off, the microphone is off. Off
    /// by default, like `tts_on`.
    #[serde(default)]
    pub stt_on: bool,
    /// How the next change of scene is drawn.
    #[serde(default)]
    pub transition: Transition,
    /// What bounds the camera's zoom.
    #[serde(default)]
    pub zoom_limit: ZoomLimit,
    /// The frame watch's plate schedule, which only the host sets (no change of the page's names
    /// it), for the toolbar's plate timer.
    #[serde(default)]
    pub plate: PlateTimer,
}

/// The frame watch's plate schedule as the host last heard it from `persona watch`, for the
/// toolbar's plate timer: the page shows it and works out no schedule of its own. Instants are on
/// the playground's clock; `now_ms` is that clock as of the reply (`ControlsReply::at`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlateTimer {
    /// Whether a watch is running.
    pub running: bool,
    /// The schedule's interval; zero when it makes none.
    pub every_ms: u64,
    /// When the next scheduled frame is due, while one may start.
    pub due_at_ms: Option<u64>,
    /// Whether a frame is being made.
    pub making: bool,
    /// The last frame's newest moment.
    pub last: Option<PlateFrame>,
    /// How many slots of the schedule came while no frame could start.
    #[serde(default)]
    pub missed: u32,
    /// When the last plate of the room went up, for the gap to the next.
    #[serde(default)]
    pub room_shown_ms: Option<u64>,
    #[serde(default)]
    pub now_ms: u64,
}

impl PlateTimer {
    /// The timer as a reader should see it `now_ms` after the host started: only a running watch
    /// carries the clock, as only a move in the scene says how long ago it began, so a reply with
    /// nothing running reads the same whenever it is asked.
    pub fn at(&self, now_ms: u64) -> Self {
        Self {
            now_ms: if self.running { now_ms } else { 0 },
            ..self.clone()
        }
    }

    /// The toolbar's words for the timer `elapsed_ms` after the reply it came in: how long until
    /// the next scheduled frame (whole seconds, counted down), or that one is being made, or that
    /// no watch runs; then the last frame's kind, moment and age.
    pub fn shown_text(&self, elapsed_ms: u64) -> String {
        if !self.running {
            return "Plate: off".to_owned();
        }
        let now_ms = self.now_ms.saturating_add(elapsed_ms);
        let mut text = if self.every_ms == 0 {
            "Plate: no schedule".to_owned()
        } else {
            match self.due_at_ms {
                Some(due) if due >= now_ms => {
                    format!("Plate in {}", minutes((due - now_ms).div_ceil(1_000)))
                }
                Some(due) => format!("Plate due {} ago", minutes((now_ms - due) / 1_000)),
                None => "Plate: waiting".to_owned(),
            }
        };
        if self.making {
            text.push_str(" \u{b7} generating\u{2026}");
        }
        if self.missed > 0 {
            text.push_str(&format!(" \u{b7} {} missed", self.missed));
        }
        let Some(last) = &self.last else {
            return text;
        };
        let kind = match last.reason {
            FrameReason::Line => "triggered",
            FrameReason::Schedule => "scheduled",
            FrameReason::Inspection => "inspection insert",
            FrameReason::Focus => "focus insert",
            FrameReason::Declared => "declared",
        };
        format!(
            "{text} \u{b7} last {kind} {} {} ago",
            last.moment.key(),
            minutes(now_ms.saturating_sub(last.at_ms) / 1_000)
        )
    }
}

/// `seconds` as `m:ss`.
fn minutes(seconds: u64) -> String {
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

/// A frame of the watch at one of its moments: its name, why it was made and when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlateFrame {
    pub frame: String,
    pub reason: FrameReason,
    pub moment: FrameMoment,
    pub at_ms: u64,
}

/// Why a frame was made: a line's change past the gate, the schedule, a close look the judge saw
/// begin, a clue's focus, or an action a character declared they are doing right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrameReason {
    Line,
    Schedule,
    Inspection,
    Focus,
    Declared,
}

impl FrameReason {
    pub fn key(self) -> &'static str {
        match self {
            Self::Line => "line",
            Self::Schedule => "schedule",
            Self::Inspection => "inspection",
            Self::Focus => "focus",
            Self::Declared => "declared",
        }
    }

    /// Whether the frame is an insert (a cut to one thing) rather than a frame of the room.
    pub fn is_insert(self) -> bool {
        matches!(self, Self::Inspection | Self::Focus)
    }
}

/// A moment of a frame the watch makes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrameMoment {
    /// It began to be made.
    Started,
    /// It went on screen.
    Shown,
    /// Making it failed.
    Failed,
    /// An insert gave the screen back to the room.
    Ended,
}

impl FrameMoment {
    pub fn key(self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::Shown => "shown",
            Self::Failed => "failed",
            Self::Ended => "ended",
        }
    }
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
            show_sam: false,
            tts_on: false,
            stt_on: false,
            transition: Transition::default(),
            zoom_limit: ZoomLimit::default(),
            plate: PlateTimer::default(),
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
            controls: Controls {
                plate: self.controls.plate.at(now_ms),
                ..self.controls.clone()
            },
            ..self.clone()
        }
    }
}

/// The host's answer to a change of what the playground shows (`path::CONTROLS_CHANGE`), as far as
/// the page reads it: the dev reply (`dev_reply::HostReply`) with the controls after the change, or
/// the host's refusal. The page shows the host's sentence and does not read the typed `error`,
/// which is for the dev CLI.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(transparent)]
pub struct ControlsChangeReply(HostReply<Box<ControlsReply>>);

impl ControlsChangeReply {
    /// The controls after the change, or the host's sentence for its refusal.
    pub fn into_result(self) -> Result<ControlsReply, String> {
        self.0
            .into_result()
            .map(|controls| *controls)
            .map_err(|rejection| rejection.message)
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
    /// Turns the voicing of lines on or off.
    Tts(bool),
    /// Turns the taking of spoken questions on or off.
    Stt(bool),
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
            ControlsChange::Tts(on) => self.tts_on = on,
            ControlsChange::Stt(on) => self.stt_on = on,
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
