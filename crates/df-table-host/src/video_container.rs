//! What a video file really is and how long it plays, read from its own bytes.
//!
//! The library never trusts a file's extension: an mp4 is an `ftyp` box with a known major brand at
//! offset 4, a webm is the EBML header with the document type `webm`. The duration comes from the
//! container (the mp4 `moov/mvhd` box, the webm Segment Info `Duration` with its `TimecodeScale`),
//! so no decoder is needed. The reader works on a `Read + Seek` source and reads only box and
//! element headers and the few payload bytes it needs (never a whole payload), so a file of any
//! size costs a bounded amount of memory and at most `MediaLimits::maximum_container_elements`
//! headers of work. It never panics on malformed input and refuses a file whose duration it cannot
//! read.
//!
//! The duration is the header's claim: a file can state one second and play longer, so the limit on
//! length checks what the container says, not the media (owner decision, 2026-11-02).

use std::fmt;
use std::io::{ErrorKind, Read, Seek, SeekFrom};
use std::ops::Range;
use std::time::Duration;

use crate::media_ingest::MediaLimits;

const MP4_BOX_HEADER_BYTES: u64 = 8;
const MP4_LARGE_BOX_HEADER_BYTES: u64 = 16;
const MP4_BRANDS: [&[u8; 4]; 12] = [
    b"isom", b"iso2", b"iso3", b"iso4", b"iso5", b"iso6", b"mp41", b"mp42", b"avc1", b"M4V ",
    b"M4VH", b"dash",
];
const MP4_MOVIE_HEADER_V0_BYTES: usize = 20;
const MP4_MOVIE_HEADER_V1_BYTES: usize = 32;
/// Bytes of the file's start that name its type: a box size, `ftyp` and the major brand.
const SNIFFED_PREFIX_BYTES: u64 = 12;

const EBML_HEADER_ID: u32 = 0x1A45_DFA3;
const EBML_DOC_TYPE_ID: u32 = 0x4282;
const WEBM_SEGMENT_ID: u32 = 0x1853_8067;
const WEBM_INFO_ID: u32 = 0x1549_A966;
const WEBM_TIMECODE_SCALE_ID: u32 = 0x002A_D7B1;
const WEBM_DURATION_ID: u32 = 0x4489;
/// An element id is at most 4 bytes and a size at most 8.
const EBML_HEADER_BYTES: u64 = 12;
const EBML_NUMBER_BYTES: u64 = 8;
/// Nanoseconds per timecode tick when the file does not say (the Matroska default).
const WEBM_DEFAULT_TIMECODE_SCALE: u64 = 1_000_000;
const WEBM_DOC_TYPE: &[u8] = b"webm";
/// Bytes of a DocType the refusal keeps; a longer one is never read past this.
const MAXIMUM_DOC_TYPE_BYTES: u64 = 24;
/// Bytes at the start of a file that the description of an unknown file looks at.
const DESCRIBED_PREFIX_BYTES: u64 = 512;

/// A video container the library accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VideoFormat {
    Mp4,
    Webm,
}

impl VideoFormat {
    /// The format a file name claims, or `None` for a name that is not a video name.
    pub(crate) fn from_name(name: &str) -> Option<Self> {
        let (_, extension) = name.rsplit_once('.')?;
        match extension.to_ascii_lowercase().as_str() {
            "mp4" => Some(Self::Mp4),
            "webm" => Some(Self::Webm),
            _ => None,
        }
    }

    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
            Self::Webm => "webm",
        }
    }
}

/// What a video file says about itself. Poster and flash-check results belong here when the
/// library records them (O.15 V1 and V5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct VideoInfo {
    pub(crate) duration: Duration,
}

/// What a file that is not the claimed video turned out to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FoundContent {
    Mp4Video,
    WebmVideo,
    /// An ISO media file whose major brand is not a video brand.
    IsoMediaBrand([u8; 4]),
    IsoMediaNoBrand,
    /// An EBML file of another type; at most `MAXIMUM_DOC_TYPE_BYTES` of the DocType.
    EbmlDocType(Vec<u8>),
    EbmlNoDocType,
    Png,
    Jpeg,
    Gif,
    Riff,
    Ogg,
    SvgOrXml,
    Html,
    PlainText,
    Binary,
}

impl fmt::Display for FoundContent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Mp4Video => formatter.write_str("an mp4 video"),
            Self::WebmVideo => formatter.write_str("a webm video"),
            Self::IsoMediaBrand(brand) => write!(
                formatter,
                "an ISO media file with the brand '{}', which is not a supported video brand",
                printable(brand)
            ),
            Self::IsoMediaNoBrand => formatter.write_str("an ISO media file with no brand"),
            Self::EbmlDocType(doc_type) => {
                write!(
                    formatter,
                    "an EBML file of type '{}', not webm",
                    printable(doc_type)
                )
            }
            Self::EbmlNoDocType => {
                formatter.write_str("an EBML file with no readable document type")
            }
            Self::Png => formatter.write_str("a PNG picture"),
            Self::Jpeg => formatter.write_str("a JPEG picture"),
            Self::Gif => formatter.write_str("a GIF picture"),
            Self::Riff => formatter.write_str("a RIFF file (a WebP picture or a WAV sound)"),
            Self::Ogg => formatter.write_str("an Ogg audio file"),
            Self::SvgOrXml => formatter.write_str("an SVG or XML document"),
            Self::Html => formatter.write_str("an HTML document"),
            Self::PlainText => formatter.write_str("plain text"),
            Self::Binary => formatter.write_str("data that is not a video"),
        }
    }
}

/// Why the duration of a video of the right format cannot be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DurationCause {
    NoMovieBox,
    NoMovieHeader,
    MovieHeaderCutShort,
    MovieHeaderUnknownVersion,
    ZeroTimescale,
    /// Zero, not a number or negative: a fragmented mp4 and a live or recorded webm write this.
    ZeroOrUnknownDuration,
    DurationOutOfRange,
    NoSegment,
    NoInfo,
    NoDuration,
    InvalidTimecodeScale,
    TooManyElements,
}

impl fmt::Display for DurationCause {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NoMovieBox => "the file has no complete moov box",
            Self::NoMovieHeader => "the moov box has no mvhd box",
            Self::MovieHeaderCutShort => "the mvhd box is cut short",
            Self::MovieHeaderUnknownVersion => "the mvhd box has an unknown version",
            Self::ZeroTimescale => "the mvhd timescale is zero",
            Self::ZeroOrUnknownDuration => {
                "the duration is zero or unknown (a fragmented mp4 or a recorded webm)"
            }
            Self::DurationOutOfRange => "the duration does not fit a duration",
            Self::NoSegment => "the file has no Segment",
            Self::NoInfo => "the Segment has no Info element",
            Self::NoDuration => "the Info element has no readable Duration",
            Self::InvalidTimecodeScale => "the TimecodeScale is zero or cannot be read",
            Self::TooManyElements => "the file has too many elements to search for its duration",
        })
    }
}

/// Why the source is not a readable video of the claimed format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum VideoError {
    /// The bytes are something else.
    WrongContent { found: FoundContent },
    /// The bytes are the right format but the duration cannot be read.
    DurationUnreadable { cause: DurationCause },
    /// The source could not be read.
    ReadFailed(ErrorKind),
}

impl fmt::Display for VideoError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongContent { found } => write!(formatter, "the file is {found}"),
            Self::DurationUnreadable { cause } => {
                write!(formatter, "the video's length cannot be read: {cause}")
            }
            Self::ReadFailed(kind) => write!(formatter, "the video could not be read: {kind}"),
        }
    }
}

impl std::error::Error for VideoError {}

fn unreadable(cause: DurationCause) -> VideoError {
    VideoError::DurationUnreadable { cause }
}

fn read_failed(error: std::io::Error) -> VideoError {
    VideoError::ReadFailed(error.kind())
}

/// `None` is "nothing there": the end of the boxes, or a box that does not fit.
type Found<T> = Result<Option<T>, VideoError>;

/// A source and its length, read a few bytes at a time.
struct Window<'a, S> {
    source: &'a mut S,
    length: u64,
}

impl<S: Read + Seek> Window<'_, S> {
    /// Up to `wanted` bytes at `offset`, stopping at `limit`; `None` when none are left.
    fn read_prefix(&mut self, offset: u64, wanted: u64, limit: u64) -> Found<Vec<u8>> {
        let end = limit.min(self.length);
        let count = wanted.min(end.saturating_sub(offset));
        if count == 0 {
            return Ok(None);
        }
        self.source
            .seek(SeekFrom::Start(offset))
            .map_err(read_failed)?;
        let mut bytes = Vec::new();
        (&mut *self.source)
            .take(count)
            .read_to_end(&mut bytes)
            .map_err(read_failed)?;
        Ok(Some(bytes))
    }
}

/// Reads `source` as a video of `claimed` format: its content type must match, and it must have a
/// readable, non-zero duration.
pub(crate) fn inspect_video<S: Read + Seek>(
    source: &mut S,
    claimed: VideoFormat,
    limits: &MediaLimits,
) -> Result<VideoInfo, VideoError> {
    let length = source.seek(SeekFrom::End(0)).map_err(read_failed)?;
    let mut window = Window { source, length };
    match (claimed, sniff(&mut window, limits)?) {
        (VideoFormat::Mp4, Sniffed::Mp4) => mp4_duration(&mut window, limits),
        (VideoFormat::Webm, Sniffed::Webm { header_end }) => {
            webm_duration(&mut window, header_end, limits)
        }
        (_, Sniffed::Mp4) => Err(wrong(FoundContent::Mp4Video)),
        (_, Sniffed::Webm { .. }) => Err(wrong(FoundContent::WebmVideo)),
        (_, Sniffed::Other(found)) => Err(wrong(found)),
    }
}

fn wrong(found: FoundContent) -> VideoError {
    VideoError::WrongContent { found }
}

enum Sniffed {
    Mp4,
    Webm { header_end: u64 },
    Other(FoundContent),
}

fn sniff<S: Read + Seek>(
    window: &mut Window<'_, S>,
    limits: &MediaLimits,
) -> Result<Sniffed, VideoError> {
    let length = window.length;
    let prefix = window
        .read_prefix(0, SNIFFED_PREFIX_BYTES, length)?
        .unwrap_or_default();
    if prefix.get(4..8) == Some(b"ftyp") {
        return Ok(match prefix.get(8..12) {
            Some(brand) if MP4_BRANDS.iter().any(|known| known.as_slice() == brand) => Sniffed::Mp4,
            Some(brand) => Sniffed::Other(match <[u8; 4]>::try_from(brand) {
                Ok(brand) => FoundContent::IsoMediaBrand(brand),
                Err(_) => FoundContent::IsoMediaNoBrand,
            }),
            None => Sniffed::Other(FoundContent::IsoMediaNoBrand),
        });
    }
    if prefix.starts_with(&EBML_HEADER_ID.to_be_bytes()) {
        return sniff_ebml(window, limits);
    }
    describe_other(window).map(Sniffed::Other)
}

/// A file that starts with the EBML header: webm when its DocType is exactly `webm`. The DocType
/// is compared and kept at most `MAXIMUM_DOC_TYPE_BYTES` long, however long the file claims it is.
fn sniff_ebml<S: Read + Seek>(
    window: &mut Window<'_, S>,
    limits: &MediaLimits,
) -> Result<Sniffed, VideoError> {
    let length = window.length;
    let Some(header) = find_ebml_element(window, 0, length, EBML_HEADER_ID, limits)? else {
        return Ok(Sniffed::Other(FoundContent::EbmlNoDocType));
    };
    let Some(doc_type) =
        find_ebml_element(window, header.start, header.end, EBML_DOC_TYPE_ID, limits)?
    else {
        return Ok(Sniffed::Other(FoundContent::EbmlNoDocType));
    };
    let doc_type_bytes = doc_type.end.saturating_sub(doc_type.start);
    let kept = window
        .read_prefix(doc_type.start, MAXIMUM_DOC_TYPE_BYTES, doc_type.end)?
        .unwrap_or_default();
    if doc_type_bytes == u64::try_from(WEBM_DOC_TYPE.len()).unwrap_or(u64::MAX)
        && kept == WEBM_DOC_TYPE
    {
        Ok(Sniffed::Webm {
            header_end: header.end,
        })
    } else {
        Ok(Sniffed::Other(FoundContent::EbmlDocType(kept)))
    }
}

fn printable(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|&byte| {
            if byte.is_ascii_graphic() || byte == b' ' {
                char::from(byte)
            } else {
                '?'
            }
        })
        .collect()
}

fn describe_other<S: Read + Seek>(window: &mut Window<'_, S>) -> Result<FoundContent, VideoError> {
    let length = window.length;
    let prefix = window
        .read_prefix(0, DESCRIBED_PREFIX_BYTES, length)?
        .unwrap_or_default();
    let signatures: [(&[u8], FoundContent); 5] = [
        (b"\x89PNG", FoundContent::Png),
        (b"\xFF\xD8\xFF", FoundContent::Jpeg),
        (b"GIF8", FoundContent::Gif),
        (b"RIFF", FoundContent::Riff),
        (b"OggS", FoundContent::Ogg),
    ];
    if let Some((_, found)) = signatures
        .into_iter()
        .find(|(signature, _)| prefix.starts_with(signature))
    {
        return Ok(found);
    }
    let Some(text) = text_prefix(&prefix) else {
        return Ok(FoundContent::Binary);
    };
    let lowered = text.trim_start().to_ascii_lowercase();
    Ok(
        if lowered.starts_with("<svg") || lowered.starts_with("<?xml") {
            FoundContent::SvgOrXml
        } else if lowered.starts_with("<!doctype html") || lowered.starts_with("<html") {
            FoundContent::Html
        } else {
            FoundContent::PlainText
        },
    )
}

/// The prefix as text. A character cut in two by the end of the prefix does not make it binary.
fn text_prefix(prefix: &[u8]) -> Option<&str> {
    match std::str::from_utf8(prefix) {
        Ok(text) => Some(text),
        Err(error) if error.error_len().is_none() => {
            std::str::from_utf8(prefix.get(..error.valid_up_to())?).ok()
        }
        Err(_) => None,
    }
}

// ---- mp4

struct Mp4Box {
    kind: [u8; 4],
    payload_start: u64,
    end: u64,
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    let end = offset.checked_add(4)?;
    Some(u32::from_be_bytes(bytes.get(offset..end)?.try_into().ok()?))
}

fn read_u64(bytes: &[u8], offset: usize) -> Option<u64> {
    let end = offset.checked_add(8)?;
    Some(u64::from_be_bytes(bytes.get(offset..end)?.try_into().ok()?))
}

/// The box starting at `offset` inside `..limit`, or `None` at the end or at a box that does not
/// fit (a truncated or malformed box ends the walk).
fn mp4_box_at<S: Read + Seek>(
    window: &mut Window<'_, S>,
    offset: u64,
    limit: u64,
) -> Found<Mp4Box> {
    let Some(header) = window.read_prefix(offset, MP4_LARGE_BOX_HEADER_BYTES, limit)? else {
        return Ok(None);
    };
    let Some(size) = read_u32(&header, 0) else {
        return Ok(None);
    };
    let Some(kind) = header
        .get(4..8)
        .and_then(|bytes| <[u8; 4]>::try_from(bytes).ok())
    else {
        return Ok(None);
    };
    let (header_bytes, total) = match size {
        0 => (MP4_BOX_HEADER_BYTES, limit.saturating_sub(offset)),
        1 => match read_u64(&header, 8) {
            Some(total) => (MP4_LARGE_BOX_HEADER_BYTES, total),
            None => return Ok(None),
        },
        _ => (MP4_BOX_HEADER_BYTES, u64::from(size)),
    };
    let Some(end) = offset.checked_add(total) else {
        return Ok(None);
    };
    if total < header_bytes || end > limit {
        return Ok(None);
    }
    Ok(Some(Mp4Box {
        kind,
        payload_start: offset.saturating_add(header_bytes),
        end,
    }))
}

/// The payload range of the first box of `kind` among the boxes in `start..limit`.
fn find_mp4_box<S: Read + Seek>(
    window: &mut Window<'_, S>,
    start: u64,
    limit: u64,
    kind: &[u8; 4],
    limits: &MediaLimits,
) -> Found<Range<u64>> {
    let mut offset = start;
    for _ in 0..limits.maximum_container_elements {
        let Some(found) = mp4_box_at(window, offset, limit)? else {
            return Ok(None);
        };
        if &found.kind == kind {
            return Ok(Some(found.payload_start..found.end));
        }
        offset = found.end;
    }
    Err(unreadable(DurationCause::TooManyElements))
}

fn mp4_duration<S: Read + Seek>(
    window: &mut Window<'_, S>,
    limits: &MediaLimits,
) -> Result<VideoInfo, VideoError> {
    let length = window.length;
    let movie = find_mp4_box(window, 0, length, b"moov", limits)?
        .ok_or(unreadable(DurationCause::NoMovieBox))?;
    let header = find_mp4_box(window, movie.start, movie.end, b"mvhd", limits)?
        .ok_or(unreadable(DurationCause::NoMovieHeader))?;
    let payload = window
        .read_prefix(header.start, 32, header.end)?
        .unwrap_or_default();
    let (timescale, duration_units, unknown_units) = match payload.first() {
        Some(0) if payload.len() >= MP4_MOVIE_HEADER_V0_BYTES => (
            read_u32(&payload, 12),
            read_u32(&payload, 16).map(u64::from),
            u64::from(u32::MAX),
        ),
        Some(1) if payload.len() >= MP4_MOVIE_HEADER_V1_BYTES => {
            (read_u32(&payload, 20), read_u64(&payload, 24), u64::MAX)
        }
        Some(0 | 1) => return Err(unreadable(DurationCause::MovieHeaderCutShort)),
        _ => return Err(unreadable(DurationCause::MovieHeaderUnknownVersion)),
    };
    let (Some(timescale), Some(units)) = (timescale, duration_units) else {
        return Err(unreadable(DurationCause::MovieHeaderCutShort));
    };
    if timescale == 0 {
        return Err(unreadable(DurationCause::ZeroTimescale));
    }
    if units == 0 || units == unknown_units {
        return Err(unreadable(DurationCause::ZeroOrUnknownDuration));
    }
    let milliseconds = (u128::from(units) * 1000).div_ceil(u128::from(timescale));
    let milliseconds =
        u64::try_from(milliseconds).map_err(|_| unreadable(DurationCause::DurationOutOfRange))?;
    Ok(VideoInfo {
        duration: Duration::from_millis(milliseconds),
    })
}

// ---- webm

struct EbmlElement {
    id: u32,
    payload_start: u64,
    end: u64,
}

/// The element id at the start of `bytes` (marker bits kept, as the specification writes ids) and
/// its length.
fn ebml_id(bytes: &[u8]) -> Option<(u32, usize)> {
    let first = *bytes.first()?;
    let length = usize::try_from(first.leading_zeros()).ok()? + 1;
    if length > 4 {
        return None;
    }
    let id = bytes
        .get(..length)?
        .iter()
        .fold(0_u32, |value, &byte| (value << 8) | u32::from(byte));
    Some((id, length))
}

/// The size at the start of `bytes`: `None` inside means "unknown size", and its length.
fn ebml_size(bytes: &[u8]) -> Option<(Option<u64>, usize)> {
    let first = *bytes.first()?;
    let length = usize::try_from(first.leading_zeros()).ok()? + 1;
    if length > 8 {
        return None;
    }
    let raw = bytes.get(..length)?;
    let marker_mask = 0xFF_u8
        .checked_shr(u32::try_from(length).ok()?)
        .unwrap_or(0);
    let value = raw.iter().enumerate().fold(0_u64, |value, (index, &byte)| {
        let byte = if index == 0 { byte & marker_mask } else { byte };
        (value << 8) | u64::from(byte)
    });
    let all_ones = (1_u64 << (7 * length)) - 1;
    Some(((value != all_ones).then_some(value), length))
}

/// The element at `offset`; an element that runs past `limit` (or has unknown size) is cut to it.
fn ebml_element_at<S: Read + Seek>(
    window: &mut Window<'_, S>,
    offset: u64,
    limit: u64,
) -> Found<EbmlElement> {
    let Some(header) = window.read_prefix(offset, EBML_HEADER_BYTES, limit)? else {
        return Ok(None);
    };
    let Some((id, id_length)) = ebml_id(&header) else {
        return Ok(None);
    };
    let Some((size, size_length)) = header.get(id_length..).and_then(ebml_size) else {
        return Ok(None);
    };
    let header_length = u64::try_from(id_length + size_length).unwrap_or(u64::MAX);
    let Some(payload_start) = offset.checked_add(header_length) else {
        return Ok(None);
    };
    if payload_start > limit {
        return Ok(None);
    }
    let end = match size {
        Some(size) => match payload_start.checked_add(size) {
            Some(end) => end.min(limit),
            None => return Ok(None),
        },
        None => limit,
    };
    Ok(Some(EbmlElement {
        id,
        payload_start,
        end,
    }))
}

fn find_ebml_element<S: Read + Seek>(
    window: &mut Window<'_, S>,
    start: u64,
    limit: u64,
    id: u32,
    limits: &MediaLimits,
) -> Found<Range<u64>> {
    let mut offset = start;
    for _ in 0..limits.maximum_container_elements {
        let Some(element) = ebml_element_at(window, offset, limit)? else {
            return Ok(None);
        };
        if element.id == id {
            return Ok(Some(element.payload_start..element.end));
        }
        offset = element.end;
    }
    Err(unreadable(DurationCause::TooManyElements))
}

fn ebml_unsigned(payload: &[u8]) -> Option<u64> {
    if payload.is_empty() || payload.len() > 8 {
        return None;
    }
    Some(
        payload
            .iter()
            .fold(0_u64, |value, &byte| (value << 8) | u64::from(byte)),
    )
}

fn ebml_float(payload: &[u8]) -> Option<f64> {
    match payload.len() {
        4 => Some(f64::from(f32::from_be_bytes(payload.try_into().ok()?))),
        8 => Some(f64::from_be_bytes(payload.try_into().ok()?)),
        _ => None,
    }
}

/// The payload of a number element, read only when it is at most `EBML_NUMBER_BYTES` long.
fn read_ebml_number<S: Read + Seek>(
    window: &mut Window<'_, S>,
    range: &Range<u64>,
) -> Found<Vec<u8>> {
    if range.end.saturating_sub(range.start) > EBML_NUMBER_BYTES {
        return Ok(None);
    }
    window.read_prefix(range.start, EBML_NUMBER_BYTES, range.end)
}

fn webm_duration<S: Read + Seek>(
    window: &mut Window<'_, S>,
    header_end: u64,
    limits: &MediaLimits,
) -> Result<VideoInfo, VideoError> {
    let length = window.length;
    let segment = find_ebml_element(window, header_end, length, WEBM_SEGMENT_ID, limits)?
        .ok_or(unreadable(DurationCause::NoSegment))?;
    let info = find_ebml_element(window, segment.start, segment.end, WEBM_INFO_ID, limits)?
        .ok_or(unreadable(DurationCause::NoInfo))?;
    let duration_range = find_ebml_element(window, info.start, info.end, WEBM_DURATION_ID, limits)?
        .ok_or(unreadable(DurationCause::NoDuration))?;
    let duration_ticks = read_ebml_number(window, &duration_range)?
        .as_deref()
        .and_then(ebml_float)
        .ok_or(unreadable(DurationCause::NoDuration))?;
    let timecode_scale =
        match find_ebml_element(window, info.start, info.end, WEBM_TIMECODE_SCALE_ID, limits)? {
            Some(range) => read_ebml_number(window, &range)?
                .as_deref()
                .and_then(ebml_unsigned)
                .ok_or(unreadable(DurationCause::InvalidTimecodeScale))?,
            None => WEBM_DEFAULT_TIMECODE_SCALE,
        };
    // Matroska writers use 1_000_000 (1 ms a tick) or less. A scale past u32::MAX (a tick of
    // more than four seconds) is refused so that the float conversion below is exact.
    let timecode_scale = u32::try_from(timecode_scale)
        .ok()
        .filter(|&scale| scale != 0)
        .ok_or(unreadable(DurationCause::InvalidTimecodeScale))?;
    let seconds = duration_ticks * f64::from(timecode_scale) / 1_000_000_000.0;
    if seconds.is_nan() || seconds <= 0.0 {
        return Err(unreadable(DurationCause::ZeroOrUnknownDuration));
    }
    let duration = Duration::try_from_secs_f64(seconds)
        .map_err(|_| unreadable(DurationCause::DurationOutOfRange))?;
    Ok(VideoInfo { duration })
}
