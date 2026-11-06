//! What the playground knows about media files: which names are acceptable, which are images,
//! which are sound and which are video, and how a page reports back what it displayed and played.
//!
//! The host serves the files and the page shows and plays them; this module is the shared
//! vocabulary, so both agree on what a name may look like and a person or an agent driving the dev
//! CLI can ask what actually happened on a screen. It never reads a file and never touches the
//! browser.

use serde::{Deserialize, Serialize};

use crate::limits::{
    MAXIMUM_MEDIA_NAME_BYTES, MAXIMUM_REPORT_BYTES, MAXIMUM_REPORTED_ERROR_CHARACTERS,
    MAXIMUM_REPORTED_ERRORS, MAXIMUM_REPORTED_IMAGES,
};

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
    /// What went wrong lately, newest last: a file that would not load, a sound the browser
    /// refused. At most `MAXIMUM_REPORTED_ERRORS`.
    pub errors: Vec<String>,
}

impl ClientReport {
    /// The size of the report as the host would hold it, in bytes of text.
    fn held_bytes(&self) -> usize {
        self.errors.iter().map(String::len).sum::<usize>()
            + self.images_shown.iter().map(String::len).sum::<usize>()
            + self.music.as_ref().map_or(0, String::len)
    }

    /// A report held to its bounds by design, so a table of pages cannot grow without limit: each
    /// text is cut to its length, the errors to their count keeping the newest, the pictures to
    /// their count, and the whole to `MAXIMUM_REPORT_BYTES` by letting the oldest errors and then
    /// the last pictures go.
    pub fn bounded(mut self) -> Self {
        let cut = |text: &mut String, limit: usize| {
            if let Some((index, _)) = text.char_indices().nth(limit) {
                text.truncate(index);
            }
        };
        if self.errors.len() > MAXIMUM_REPORTED_ERRORS {
            let excess = self.errors.len() - MAXIMUM_REPORTED_ERRORS;
            self.errors.drain(..excess);
        }
        self.errors
            .iter_mut()
            .for_each(|error| cut(error, MAXIMUM_REPORTED_ERROR_CHARACTERS));
        self.images_shown.truncate(MAXIMUM_REPORTED_IMAGES);
        self.images_shown
            .iter_mut()
            .for_each(|name| cut(name, MAXIMUM_MEDIA_NAME_BYTES));
        if let Some(music) = &mut self.music {
            cut(music, MAXIMUM_MEDIA_NAME_BYTES);
        }
        while self.held_bytes() > MAXIMUM_REPORT_BYTES {
            if !self.errors.is_empty() {
                self.errors.remove(0);
            } else if self.images_shown.pop().is_none() {
                break;
            }
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
