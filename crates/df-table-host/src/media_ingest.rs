//! The checks a file passes before it enters the media library: size, room in the library, and
//! for a video its real type and duration read from the bytes (never the extension). Pictures
//! and sounds are only size-checked. It decides nothing about what is shown or played, and writes
//! nothing.
//!
//! Every media limit is in `MediaLimits::DEFAULT`, one block (owner decision, 2026-11-02: large
//! defaults first, scoped down on a measured need). Memory safety does not rest on these numbers:
//! an upload is decoded to a staged file in pieces, a file is read through a seekable handle a
//! few bytes at a time, and a file is served in chunks.

use std::fmt;
use std::io::{ErrorKind, Read, Seek, SeekFrom};
use std::time::Duration;

use axum::http::StatusCode;
use df_playground::media::{
    AUDIO_EXTENSIONS, IMAGE_EXTENSIONS, MediaEntry, MediaKind, VIDEO_EXTENSIONS, media_kind,
};

use crate::media_upload::EnvelopeCause;
use crate::video_container::{DurationCause, FoundContent, VideoError, VideoFormat, inspect_video};

const BYTES_PER_GIB: u64 = 1024 * 1024 * 1024;
/// Bytes the request carries besides the file's base64: the JSON braces, the two keys, the name
/// (with escapes) and whitespace.
const UPLOAD_ENVELOPE_BYTES: u64 = 4096;

/// The limits of the media library, all in one place.
#[derive(Debug, Clone, Copy)]
pub(crate) struct MediaLimits {
    /// Largest file of any kind, in bytes.
    pub(crate) maximum_file_bytes: u64,
    /// Longest video, as its container states it.
    pub(crate) maximum_video_duration: Duration,
    /// Most files in the library.
    pub(crate) maximum_files: usize,
    /// Most boxes or elements the container reader looks at in one file before giving up.
    pub(crate) maximum_container_elements: usize,
}

impl MediaLimits {
    pub(crate) const DEFAULT: Self = Self {
        maximum_file_bytes: 2 * BYTES_PER_GIB,
        maximum_video_duration: Duration::from_secs(12 * 60 * 60),
        maximum_files: 10_000,
        maximum_container_elements: 1_000_000,
    };

    /// Largest add request: the largest file as base64, plus the envelope.
    pub(crate) const fn maximum_upload_body_bytes(&self) -> u64 {
        self.maximum_file_bytes
            .div_ceil(3)
            .saturating_mul(4)
            .saturating_add(UPLOAD_ENVELOPE_BYTES)
    }
}

/// Largest file, in bytes.
pub const MAXIMUM_MEDIA_BYTES: u64 = MediaLimits::DEFAULT.maximum_file_bytes;
/// Longest video.
pub const MAXIMUM_VIDEO_DURATION: Duration = MediaLimits::DEFAULT.maximum_video_duration;
/// Most files in the library.
pub const MAXIMUM_MEDIA_FILES: usize = MediaLimits::DEFAULT.maximum_files;
/// Largest add request, in bytes.
pub const MAXIMUM_UPLOAD_BODY_BYTES: u64 = MediaLimits::DEFAULT.maximum_upload_body_bytes();

/// What a media file name says it is. A video carries its format, read from the name once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UploadKind {
    Image,
    Audio,
    Video(VideoFormat),
}

impl UploadKind {
    /// The kind a name claims, or `None` when it is not a plain name of a known type.
    pub(crate) fn from_name(name: &str) -> Option<Self> {
        match media_kind(name)? {
            MediaKind::Image => Some(Self::Image),
            MediaKind::Audio => Some(Self::Audio),
            MediaKind::Video => VideoFormat::from_name(name).map(Self::Video),
        }
    }
}

/// The file types the library takes, for a refusal and the help text.
pub(crate) fn kinds_help() -> String {
    format!(
        "pictures: {}; sounds: {}; videos: {}",
        IMAGE_EXTENSIONS.join(", "),
        AUDIO_EXTENSIONS.join(", "),
        VIDEO_EXTENSIONS.join(", ")
    )
}

/// Why a file is not added to the library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum IngestError {
    /// The request is not the JSON object the route takes.
    InvalidRequest(EnvelopeCause),
    InvalidMediaName {
        name: String,
    },
    InvalidBase64,
    Empty,
    TooLarge {
        maximum_bytes: u64,
    },
    LibraryFull {
        maximum_files: usize,
    },
    /// The bytes are not the video type the name claims.
    NotThatVideoType {
        claimed: VideoFormat,
        found: FoundContent,
    },
    DurationUnreadable {
        claimed: VideoFormat,
        cause: DurationCause,
    },
    TooLong {
        duration: Duration,
        maximum: Duration,
    },
    /// The staged file could not be written or read back.
    FileFailed(ErrorKind),
}

impl IngestError {
    pub(crate) fn status(&self) -> StatusCode {
        match self {
            Self::TooLarge { .. } => StatusCode::PAYLOAD_TOO_LARGE,
            Self::FileFailed(_) => StatusCode::INTERNAL_SERVER_ERROR,
            _ => StatusCode::BAD_REQUEST,
        }
    }
}

impl fmt::Display for IngestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRequest(cause) => write!(formatter, "the request is not valid: {cause}"),
            Self::InvalidMediaName { name } => write!(
                formatter,
                "'{name}' is not a media name: use a plain name ending in one of these ({})",
                kinds_help()
            ),
            Self::InvalidBase64 => formatter.write_str("the file data is not valid base64"),
            Self::Empty => formatter.write_str("the file is empty"),
            Self::TooLarge { maximum_bytes } => write!(
                formatter,
                "a media file is at most {maximum_bytes} bytes ({} GiB)",
                maximum_bytes / BYTES_PER_GIB
            ),
            Self::LibraryFull { maximum_files } => write!(
                formatter,
                "the library holds at most {maximum_files} files; remove one first"
            ),
            Self::NotThatVideoType { claimed, found } => write!(
                formatter,
                "the name says {} but the file is {found}",
                claimed.key()
            ),
            Self::DurationUnreadable { claimed, cause } => write!(
                formatter,
                "the length of this {} cannot be read ({cause}), so it is refused",
                claimed.key()
            ),
            Self::TooLong { duration, maximum } => write!(
                formatter,
                "this video is {:.1} s long; a video is at most {} s",
                duration.as_secs_f64(),
                maximum.as_secs()
            ),
            Self::FileFailed(kind) => write!(formatter, "the file could not be stored: {kind}"),
        }
    }
}

impl std::error::Error for IngestError {}

/// The video limits as help text, for the dev command line.
pub fn video_limits_text() -> String {
    format!(
        "a video is mp4 or webm (the type is read from the file, not the name), at most {} hours \
         and {} GiB",
        MAXIMUM_VIDEO_DURATION.as_secs() / 3600,
        MAXIMUM_MEDIA_BYTES / BYTES_PER_GIB
    )
}

/// The checks that need only the staged file: it is not empty, its size is within the limit and,
/// for a video, its content type and duration are what the name claims. It reads a few bytes of
/// the file, never all of it.
pub(crate) fn check_content<S: Read + Seek>(
    source: &mut S,
    upload: UploadKind,
    limits: &MediaLimits,
) -> Result<(), IngestError> {
    let length = source
        .seek(SeekFrom::End(0))
        .map_err(|error| IngestError::FileFailed(error.kind()))?;
    if length == 0 {
        return Err(IngestError::Empty);
    }
    if length > limits.maximum_file_bytes {
        return Err(IngestError::TooLarge {
            maximum_bytes: limits.maximum_file_bytes,
        });
    }
    let UploadKind::Video(claimed) = upload else {
        return Ok(());
    };
    let info = inspect_video(source, claimed, limits).map_err(|refusal| match refusal {
        VideoError::WrongContent { found } => IngestError::NotThatVideoType { claimed, found },
        VideoError::DurationUnreadable { cause } => {
            IngestError::DurationUnreadable { claimed, cause }
        }
        VideoError::ReadFailed(kind) => IngestError::FileFailed(kind),
    })?;
    if info.duration > limits.maximum_video_duration {
        return Err(IngestError::TooLong {
            duration: info.duration,
            maximum: limits.maximum_video_duration,
        });
    }
    Ok(())
}

/// Whether the library has room for `name`. Replacing a file of the same name needs none.
pub(crate) fn check_room(
    existing: &[MediaEntry],
    name: &str,
    limits: &MediaLimits,
) -> Result<(), IngestError> {
    if existing.iter().any(|entry| entry.name == name) {
        return Ok(());
    }
    if existing.len() >= limits.maximum_files {
        return Err(IngestError::LibraryFull {
            maximum_files: limits.maximum_files,
        });
    }
    Ok(())
}
