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
use crate::sound_plan::AudioPrefs;

use crate::limits::{DEFAULT_TRANSITION_MS, MAXIMUM_TRANSITION_MS, MAXIMUM_VOLUME_PERCENT};

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
    /// How loud the sound is, whether it is muted and whether the music plays. The host keeps it
    /// with the other switches, so a page that opens plays by what was last chosen. The music is
    /// off by default: a track the library lacks is composed (and paid for) only while it is on.
    #[serde(default)]
    pub audio: AudioPrefs,
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
    /// Whether a frame is being made: true while `in_flight` is not empty.
    pub making: bool,
    /// The frames being made, by name, in the order they started: each started frame is here until
    /// its one closing moment (`shown`, `failed` or `dropped`), so two made at once count two and
    /// one frame's end never clears another's. An older host sends none; `making` then says it.
    #[serde(default)]
    pub in_flight: Vec<String>,
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
    /// the next scheduled frame (whole seconds, counted down) or that none is scheduled, or that
    /// no watch runs; how long the frame being made has been going, and its kind; how long the
    /// room's plate has been up; then the last frame's kind, moment and age (left out while that
    /// last moment is the start of the frame being made, which has just been said).
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
        let age = |at_ms: u64| minutes(now_ms.saturating_sub(at_ms) / 1_000);
        let making_now = self
            .last
            .as_ref()
            .filter(|last| self.making && last.moment == FrameMoment::Started);
        if let Some(last) = making_now {
            text.push_str(&format!(
                " \u{b7} generating ({}) {}",
                last.reason.words(),
                age(last.at_ms)
            ));
        } else if self.making {
            text.push_str(" \u{b7} generating\u{2026}");
        }
        if self.in_flight.len() > 1 {
            text.push_str(&format!(" \u{b7} {} in flight", self.in_flight.len()));
        }
        if self.missed > 0 {
            text.push_str(&format!(" \u{b7} {} missed", self.missed));
        }
        let room_plate_is_last = self.last.as_ref().is_some_and(|last| {
            last.moment == FrameMoment::Shown
                && !last.reason.is_insert()
                && Some(last.at_ms) == self.room_shown_ms
        });
        if let Some(shown) = self.room_shown_ms.filter(|_| !room_plate_is_last) {
            text.push_str(&format!(" \u{b7} room plate up {}", age(shown)));
        }
        match &self.last {
            Some(last) if making_now.is_none() => format!(
                "{text} \u{b7} last {} {} {} ago",
                last.reason.words(),
                last.moment.key(),
                age(last.at_ms)
            ),
            _ => text,
        }
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
    /// A picture already in the library cut to as it is, with a caption (`@show`).
    Reveal,
    /// A sight on the way to a place, or the place itself, shown in the walk of a move.
    Walk,
}

impl FrameReason {
    pub fn key(self) -> &'static str {
        match self {
            Self::Line => "line",
            Self::Schedule => "schedule",
            Self::Inspection => "inspection",
            Self::Focus => "focus",
            Self::Declared => "declared",
            Self::Reveal => "reveal",
            Self::Walk => "walk",
        }
    }

    /// The toolbar's words for the frame's kind.
    pub fn words(self) -> &'static str {
        match self {
            Self::Line => "triggered",
            Self::Schedule => "scheduled",
            Self::Inspection => "inspection insert",
            Self::Focus => "focus insert",
            Self::Declared => "declared",
            Self::Reveal => "revealed picture",
            Self::Walk => "walk",
        }
    }

    /// Whether the frame is an insert (a cut to one thing) rather than a frame of the room.
    pub fn is_insert(self) -> bool {
        matches!(
            self,
            Self::Inspection | Self::Focus | Self::Reveal | Self::Walk
        )
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
    /// It finished without going up and without failing: another picture took its place
    /// (superseded, overtaken), it was replaced by a newer one, or the talk moved on.
    Dropped,
    /// An insert gave the screen back to the room.
    Ended,
}

impl FrameMoment {
    pub fn key(self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::Shown => "shown",
            Self::Failed => "failed",
            Self::Dropped => "dropped",
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
            audio: AudioPrefs::default(),
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
    /// Mutes or unmutes every sound the pages play.
    Sound(bool),
    /// Turns the background music on or off; with it off no track is composed either.
    Music(bool),
    /// Sets the master volume, 0 to `MAXIMUM_VOLUME_PERCENT`.
    Volume(u8),
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
    VolumeTooHigh(u8),
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
            Self::VolumeTooHigh(percent) => write!(
                formatter,
                "the volume is from 0 to {MAXIMUM_VOLUME_PERCENT} percent, not {percent}"
            ),
        }
    }
}

impl std::error::Error for ControlsError {}

/// An on/off switch of the toolbar. A click on one asks for the change `toggled` gives, and the
/// switch is drawn lit exactly when `is_on` says so, so what is shown is what the host holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Switch {
    Unavailable,
    Grid,
    Sam,
    Sound,
    Music,
    Tts,
    Stt,
}

impl Switch {
    pub const ALL: [Self; 7] = [
        Self::Unavailable,
        Self::Grid,
        Self::Sam,
        Self::Sound,
        Self::Music,
        Self::Tts,
        Self::Stt,
    ];

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|switch| switch.key() == key)
    }

    /// The word after `toggle:` in the `data-action` of the switch's button.
    pub fn key(self) -> &'static str {
        match self {
            Self::Unavailable => "unavailable",
            Self::Grid => "grid",
            Self::Sam => "sam",
            Self::Sound => "sound",
            Self::Music => "music",
            Self::Tts => "tts",
            Self::Stt => "stt",
        }
    }

    /// The words on the switch's button.
    pub fn words(self) -> &'static str {
        match self {
            Self::Unavailable => "Unavailable",
            Self::Grid => "Grid",
            Self::Sam => "SAM",
            Self::Sound => "Sound",
            Self::Music => "Music",
            Self::Tts => "TTS",
            Self::Stt => "STT",
        }
    }

    /// Whether the setting the switch stands for is on.
    pub fn is_on(self, controls: &Controls) -> bool {
        match self {
            Self::Unavailable => controls.show_unavailable,
            Self::Grid => controls.show_grid,
            Self::Sam => controls.show_sam,
            Self::Sound => controls.audio.sound_on,
            Self::Music => controls.audio.music_on,
            Self::Tts => controls.tts_on,
            Self::Stt => controls.stt_on,
        }
    }

    /// The change a click asks for: the setting the other way round from how `controls` holds it.
    pub fn toggled(self, controls: &Controls) -> ControlsChange {
        let on = !self.is_on(controls);
        match self {
            Self::Unavailable => ControlsChange::ShowUnavailable(on),
            Self::Grid => ControlsChange::ShowGrid(on),
            Self::Sam => ControlsChange::ShowSam(on),
            Self::Sound => ControlsChange::Sound(on),
            Self::Music => ControlsChange::Music(on),
            Self::Tts => ControlsChange::Tts(on),
            Self::Stt => ControlsChange::Stt(on),
        }
    }
}

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
            ControlsChange::Sound(on) => self.audio.sound_on = on,
            ControlsChange::Music(on) => self.audio.music_on = on,
            ControlsChange::Volume(percent) => {
                if percent > MAXIMUM_VOLUME_PERCENT {
                    return Err(ControlsError::VolumeTooHigh(percent));
                }
                self.audio.volume_percent = percent;
            }
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
