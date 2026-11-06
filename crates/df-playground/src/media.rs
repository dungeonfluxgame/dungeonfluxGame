//! What the playground knows about media files: which names are acceptable, which are images,
//! which are sound and which are video, and how a page reports back what it displayed and played.
//!
//! The host serves the files and the page shows and plays them; this module is the shared
//! vocabulary, so both agree on what a name may look like and a person or an agent driving the dev
//! CLI can ask what actually happened on a screen. It never reads a file and never touches the
//! browser.

use serde::{Deserialize, Serialize};

use crate::controls::Device;
use crate::copy::ReadingPace;
use crate::limits::{
    MAXIMUM_MEDIA_NAME_BYTES, MAXIMUM_REPORT_BYTES, MAXIMUM_REPORTED_ERROR_CHARACTERS,
    MAXIMUM_REPORTED_ERRORS, MAXIMUM_REPORTED_IMAGES, MAXIMUM_TIMELINE_RECORDS,
};
use crate::timeline::{ClientError, FixtureLoadCause, LinkState, TimelineRecord, ViewportPx, cut};

/// Image file types a page can show. SVG is left out on purpose: opened on its own it can run
/// script from the host's origin.
pub const IMAGE_EXTENSIONS: [&str; 5] = ["png", "jpg", "jpeg", "webp", "gif"];
/// Sound file types a page can play.
pub const AUDIO_EXTENSIONS: [&str; 5] = ["mp3", "ogg", "wav", "m4a", "opus"];
/// Video file types the library accepts: mp4 (H.264 with AAC) and webm (VP9 with Opus).
pub const VIDEO_EXTENSIONS: [&str; 2] = ["mp4", "webm"];

/// True for a name that cannot leave its directory: letters, digits, `_`, `-` and `.`, no leading
/// dot and no `..`.
pub fn is_plain_file_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAXIMUM_MEDIA_NAME_BYTES
        && !name.starts_with('.')
        && !name.contains("..")
        && name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "_-.".contains(character))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    Image,
    Audio,
    Video,
}

impl MediaKind {
    pub const ALL: [Self; 3] = [Self::Image, Self::Audio, Self::Video];

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.key() == key)
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Audio => "audio",
            Self::Video => "video",
        }
    }
}

/// The kind of a media file from its name, or `None` when it is not a plain name of a known type.
pub fn media_kind(name: &str) -> Option<MediaKind> {
    if !is_plain_file_name(name) {
        return None;
    }
    let (_, extension) = name.rsplit_once('.')?;
    let extension = extension.to_ascii_lowercase();
    if IMAGE_EXTENSIONS.contains(&extension.as_str()) {
        Some(MediaKind::Image)
    } else if AUDIO_EXTENSIONS.contains(&extension.as_str()) {
        Some(MediaKind::Audio)
    } else if VIDEO_EXTENSIONS.contains(&extension.as_str()) {
        Some(MediaKind::Video)
    } else {
        None
    }
}

/// The media type to serve a file with, from its name.
pub fn content_type(name: &str) -> &'static str {
    let extension = name
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default();
    match extension.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "mp3" => "audio/mpeg",
        "ogg" => "audio/ogg",
        "wav" => "audio/wav",
        "m4a" => "audio/mp4",
        "opus" => "audio/ogg; codecs=opus",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        _ => "application/octet-stream",
    }
}

/// One file in the media library, as the host lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaEntry {
    pub name: String,
    pub kind: MediaKind,
    pub bytes: u64,
}

/// Whether a page may play sound yet. A browser keeps sound locked until the person clicks or
/// presses a key on the page, so an agent cannot assume a sound it asked for was heard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioState {
    Locked,
    Unlocked,
}

impl AudioState {
    pub const ALL: [Self; 2] = [Self::Locked, Self::Unlocked];

    /// The serde name.
    pub fn key(self) -> &'static str {
        match self {
            Self::Locked => "locked",
            Self::Unlocked => "unlocked",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|value| value.key() == key)
    }
}

/// What one page says it is showing and playing, sent to the host so the dev CLI can ask. An agent
/// cannot see or hear a screen; this is how it checks. A misspelt key is refused, not dropped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientReport {
    pub audio: AudioState,
    /// The newest sound cue this page has played or tried to play.
    pub last_cue: u64,
    /// The music playing now, by media name.
    pub music: Option<String>,
    /// Media shown now and loaded without error, by name.
    pub images_shown: Vec<String>,
    /// What went wrong lately, oldest first: a file that would not load, a sound the browser
    /// refused, a draw that failed. At most `MAXIMUM_REPORTED_ERRORS`.
    pub errors: Vec<ClientError>,
    /// The newest frame of the host the page drew.
    pub frame: Option<u64>,
    /// The run of the host the page follows.
    pub boot: Option<String>,
    /// How many times in a row the host could not be reached.
    pub failures: u32,
    pub link: LinkState,
    /// The revision of the playground files the page last read (`<assets>.<package>`).
    pub asset_revision: String,
    /// The fixture file the page loaded, and why it could not be shown when it could not.
    pub fixture: Option<String>,
    pub fixture_error: Option<FixtureLoadCause>,
    pub device: Device,
    pub viewport_px: Option<ViewportPx>,
    pub pace: ReadingPace,
    /// The page's clock when it made the report, in milliseconds, so the host can put the page's
    /// times on its own clock.
    pub page_clock_ms: u64,
    /// What the page started or ended showing and playing since the last report it delivered.
    pub timeline: Vec<TimelineRecord>,
}

impl Default for ClientReport {
    fn default() -> Self {
        Self {
            audio: AudioState::Locked,
            last_cue: 0,
            music: None,
            images_shown: Vec::new(),
            errors: Vec::new(),
            frame: None,
            boot: None,
            failures: 0,
            link: LinkState::Up,
            asset_revision: String::new(),
            fixture: None,
            fixture_error: None,
            device: Device::Both,
            viewport_px: None,
            pace: ReadingPace::Normal,
            page_clock_ms: 0,
            timeline: Vec::new(),
        }
    }
}

impl ClientReport {
    /// The size of the report as the host would hold it, in bytes of text.
    fn held_bytes(&self) -> usize {
        self.errors
            .iter()
            .map(|error| error.message.len())
            .sum::<usize>()
            + self.images_shown.iter().map(String::len).sum::<usize>()
            + self.music.as_ref().map_or(0, String::len)
            + self.boot.as_ref().map_or(0, String::len)
            + self.asset_revision.len()
            + self.fixture.as_ref().map_or(0, String::len)
            + self
                .timeline
                .iter()
                .map(TimelineRecord::held_bytes)
                .sum::<usize>()
    }

    /// A report held to its bounds by design, so a table of pages cannot grow without limit: each
    /// text is cut to its length, the errors to their count keeping the newest, the pictures and
    /// the timeline records to their count, and the whole to `MAXIMUM_REPORT_BYTES` by letting the
    /// oldest errors, then the last pictures and then the oldest records go.
    pub fn bounded(mut self) -> Self {
        if self.errors.len() > MAXIMUM_REPORTED_ERRORS {
            let excess = self.errors.len() - MAXIMUM_REPORTED_ERRORS;
            self.errors.drain(..excess);
        }
        self.errors
            .iter_mut()
            .for_each(|error| cut(&mut error.message, MAXIMUM_REPORTED_ERROR_CHARACTERS));
        self.images_shown.truncate(MAXIMUM_REPORTED_IMAGES);
        self.images_shown
            .iter_mut()
            .for_each(|name| cut(name, MAXIMUM_MEDIA_NAME_BYTES));
        for text in [&mut self.music, &mut self.boot, &mut self.fixture]
            .into_iter()
            .flatten()
        {
            cut(text, MAXIMUM_MEDIA_NAME_BYTES);
        }
        cut(&mut self.asset_revision, MAXIMUM_MEDIA_NAME_BYTES);
        if self.timeline.len() > MAXIMUM_TIMELINE_RECORDS {
            let excess = self.timeline.len() - MAXIMUM_TIMELINE_RECORDS;
            self.timeline.drain(..excess);
        }
        self.timeline = self
            .timeline
            .into_iter()
            .map(TimelineRecord::bounded)
            .collect();
        while self.held_bytes() > MAXIMUM_REPORT_BYTES {
            if !self.errors.is_empty() {
                self.errors.remove(0);
                continue;
            }
            if self.images_shown.pop().is_some() {
                continue;
            }
            if self.timeline.is_empty() {
                break;
            }
            self.timeline.remove(0);
        }
        self
    }
}

/// A report and who sent it, as the page posts it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientReportPost {
    /// A random name the page makes for itself when it loads, so two pages can be told apart.
    pub client: String,
    pub report: ClientReport,
}
