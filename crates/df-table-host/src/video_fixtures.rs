//! Tiny valid videos built from bytes, so the tests need no files: a container with a header and
//! a duration, and optional padding to reach a size. They are not playable; only the container
//! parts the library reads are real. Each builder says which encoder's layout it copies.

use std::io::{Read, Seek, SeekFrom};

fn mp4_box(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let size = u32::try_from(payload.len() + 8).expect("a test box fits 32 bits");
    let mut bytes = size.to_be_bytes().to_vec();
    bytes.extend_from_slice(kind);
    bytes.extend_from_slice(payload);
    bytes
}

fn ftyp(brand: &[u8; 4]) -> Vec<u8> {
    let mut payload = brand.to_vec();
    payload.extend_from_slice(&[0, 0, 2, 0]);
    payload.extend_from_slice(b"isomiso2mp41");
    mp4_box(b"ftyp", &payload)
}

fn mvhd(timescale: u32, units: u32) -> Vec<u8> {
    let mut header = vec![0_u8; 4];
    header.extend_from_slice(&[0; 8]);
    header.extend_from_slice(&timescale.to_be_bytes());
    header.extend_from_slice(&units.to_be_bytes());
    header.extend_from_slice(&[0; 80]);
    mp4_box(b"mvhd", &header)
}

/// A `moov` as `ffmpeg` writes it: the movie header, a track and a user-data box.
fn ffmpeg_moov(timescale: u32, units: u32) -> Vec<u8> {
    let mut payload = mvhd(timescale, units);
    payload.extend(mp4_box(b"trak", &[0; 96]));
    payload.extend(mp4_box(b"udta", &[0; 24]));
    mp4_box(b"moov", &payload)
}

/// An mp4 with the given major brand and a version 0 `mvhd`.
pub(crate) fn mp4_with_brand(brand: &[u8; 4], timescale: u32, units: u32) -> Vec<u8> {
    let mut bytes = ftyp(brand);
    bytes.extend(mp4_box(b"moov", &mvhd(timescale, units)));
    bytes
}

/// A valid mp4 that plays for `seconds` at a timescale of 1000.
pub(crate) fn mp4_of_seconds(seconds: u32) -> Vec<u8> {
    mp4_with_brand(b"isom", 1000, seconds * 1000)
}

/// An mp4 with a version 1 `mvhd` (64-bit times).
pub(crate) fn mp4_version_one(timescale: u32, units: u64) -> Vec<u8> {
    let mut header = vec![1_u8, 0, 0, 0];
    header.extend_from_slice(&[0; 16]);
    header.extend_from_slice(&timescale.to_be_bytes());
    header.extend_from_slice(&units.to_be_bytes());
    header.extend_from_slice(&[0; 80]);
    let mut bytes = ftyp(b"isom");
    bytes.extend(mp4_box(b"moov", &mp4_box(b"mvhd", &header)));
    bytes
}

/// The start of an mp4 that is `total_bytes` long: a one-second video and the header of an `mdat`
/// box that covers the rest of the file. The rest is padding; a file of this size is this prefix
/// followed by zeros, so a sparse file can stand for it.
pub(crate) fn mp4_of_size_prefix(total_bytes: usize) -> Vec<u8> {
    let mut bytes = mp4_of_seconds(1);
    let padding = total_bytes
        .checked_sub(bytes.len() + 8)
        .expect("the size is larger than the header");
    let size = u32::try_from(padding + 8).expect("a test box fits 32 bits");
    bytes.extend_from_slice(&size.to_be_bytes());
    bytes.extend_from_slice(b"mdat");
    bytes
}

/// An mp4 that is `total_bytes` long: a one-second video followed by an `mdat` box of padding.
pub(crate) fn mp4_of_size(total_bytes: usize) -> Vec<u8> {
    let mut bytes = mp4_of_size_prefix(total_bytes);
    bytes.resize(total_bytes, 0);
    bytes
}

/// An mp4 with an `ftyp` and a `moov` that holds no `mvhd`.
pub(crate) fn mp4_without_movie_header() -> Vec<u8> {
    let mut bytes = ftyp(b"isom");
    bytes.extend(mp4_box(b"moov", &mp4_box(b"trak", &[0; 16])));
    bytes
}

/// What `ffmpeg -i in out.mp4` writes by default: `ftyp`, `free`, `mdat`, then `moov` (the
/// movie header comes after all the media, so a reader must get past a large `mdat`).
pub(crate) fn mp4_ffmpeg_default(seconds: u32, media_bytes: usize) -> Vec<u8> {
    let mut bytes = ftyp(b"isom");
    bytes.extend(mp4_box(b"free", &[]));
    bytes.extend(mp4_box(b"mdat", &vec![0; media_bytes]));
    bytes.extend(ffmpeg_moov(1000, seconds * 1000));
    bytes
}

/// What `ffmpeg -movflags +faststart` writes: `ftyp`, `moov`, `free`, `mdat`.
pub(crate) fn mp4_ffmpeg_faststart(seconds: u32, media_bytes: usize) -> Vec<u8> {
    let mut bytes = ftyp(b"isom");
    bytes.extend(ffmpeg_moov(1000, seconds * 1000));
    bytes.extend(mp4_box(b"free", &[]));
    bytes.extend(mp4_box(b"mdat", &vec![0; media_bytes]));
    bytes
}

/// A muxer that writes a 64-bit box size (`size` field 1) for a large `mdat`, before the `moov`.
pub(crate) fn mp4_with_large_size_mdat_before_moov(seconds: u32, media_bytes: usize) -> Vec<u8> {
    let mut bytes = ftyp(b"isom");
    let total = u64::try_from(media_bytes + 16).expect("a test size fits 64 bits");
    bytes.extend_from_slice(&1_u32.to_be_bytes());
    bytes.extend_from_slice(b"mdat");
    bytes.extend_from_slice(&total.to_be_bytes());
    bytes.extend(vec![0; media_bytes]);
    bytes.extend(ffmpeg_moov(1000, seconds * 1000));
    bytes
}

/// A pipe-written file: `ftyp`, `moov`, then an `mdat` whose size field is 0 (it runs to the end).
pub(crate) fn mp4_with_open_ended_mdat_last(seconds: u32, media_bytes: usize) -> Vec<u8> {
    let mut bytes = ftyp(b"isom");
    bytes.extend(ffmpeg_moov(1000, seconds * 1000));
    bytes.extend_from_slice(&0_u32.to_be_bytes());
    bytes.extend_from_slice(b"mdat");
    bytes.extend(vec![0; media_bytes]);
    bytes
}

/// The same file with the open-ended `mdat` first: the `moov` is inside it, so it cannot be found.
pub(crate) fn mp4_with_open_ended_mdat_before_moov(seconds: u32, media_bytes: usize) -> Vec<u8> {
    let mut bytes = ftyp(b"isom");
    bytes.extend_from_slice(&0_u32.to_be_bytes());
    bytes.extend_from_slice(b"mdat");
    bytes.extend(vec![0; media_bytes]);
    bytes.extend(ffmpeg_moov(1000, seconds * 1000));
    bytes
}

/// A file whose `moov` box claims `claimed_size` bytes, more than the file holds.
pub(crate) fn mp4_with_moov_past_the_end(claimed_size: u32) -> Vec<u8> {
    let mut bytes = ftyp(b"isom");
    bytes.extend_from_slice(&claimed_size.to_be_bytes());
    bytes.extend_from_slice(b"moov");
    bytes.extend(mvhd(1000, 5000));
    bytes
}

/// A `moov` whose only child claims `claimed_size` bytes, more than the `moov` holds.
pub(crate) fn mp4_with_mvhd_past_the_moov(claimed_size: u32) -> Vec<u8> {
    let mut child = claimed_size.to_be_bytes().to_vec();
    child.extend_from_slice(b"mvhd");
    child.extend_from_slice(&[0; 40]);
    let mut bytes = ftyp(b"isom");
    bytes.extend(mp4_box(b"moov", &child));
    bytes
}

/// How an element writes its size.
#[derive(Clone, Copy)]
enum Size {
    /// The fewest bytes that hold it (what libwebm and ffmpeg write for most elements).
    Minimal,
    /// Always eight bytes (what both write for a Segment that is patched when the file closes).
    Eight,
    /// The reserved all-ones value: the length is not known (a live muxer).
    Unknown,
}

fn ebml_size(length: usize, style: Size) -> Vec<u8> {
    let length = u64::try_from(length).expect("a test length fits 64 bits");
    let width: u32 = match style {
        Size::Eight | Size::Unknown => 8,
        Size::Minimal => (1..=8)
            .find(|width| length < (1_u64 << (7 * width)) - 1)
            .expect("a test length fits eight bytes"),
    };
    let value = match style {
        Size::Unknown => (1_u64 << 56) | ((1_u64 << 56) - 1),
        _ => (1_u64 << (7 * width)) | length,
    };
    let start = 8 - usize::try_from(width).expect("a width fits");
    value.to_be_bytes()[start..].to_vec()
}

fn ebml_element_styled(id: &[u8], payload: &[u8], style: Size) -> Vec<u8> {
    let mut bytes = id.to_vec();
    bytes.extend(ebml_size(payload.len(), style));
    bytes.extend_from_slice(payload);
    bytes
}

fn ebml_element(id: &[u8], payload: &[u8]) -> Vec<u8> {
    ebml_element_styled(id, payload, Size::Eight)
}

fn minimal(id: &[u8], payload: &[u8]) -> Vec<u8> {
    ebml_element_styled(id, payload, Size::Minimal)
}

fn ebml_header(doc_type: &str) -> Vec<u8> {
    ebml_element(
        &[0x1A, 0x45, 0xDF, 0xA3],
        &ebml_element(&[0x42, 0x82], doc_type.as_bytes()),
    )
}

/// The EBML header as `ffmpeg` and libwebm write it: version fields, then the document type.
fn full_ebml_header() -> Vec<u8> {
    let mut children = minimal(&[0x42, 0x86], &[1]);
    children.extend(minimal(&[0x42, 0xF7], &[1]));
    children.extend(minimal(&[0x42, 0xF2], &[4]));
    children.extend(minimal(&[0x42, 0xF3], &[8]));
    children.extend(minimal(&[0x42, 0x82], b"webm"));
    children.extend(minimal(&[0x42, 0x87], &[4]));
    children.extend(minimal(&[0x42, 0x85], &[2]));
    minimal(&[0x1A, 0x45, 0xDF, 0xA3], &children)
}

const SEEK_HEAD_ID: [u8; 4] = [0x11, 0x4D, 0x9B, 0x74];
const VOID_ID: [u8; 1] = [0xEC];
const INFO_ID: [u8; 4] = [0x15, 0x49, 0xA9, 0x66];
const SEGMENT_ID: [u8; 4] = [0x18, 0x53, 0x80, 0x67];
const TRACKS_ID: [u8; 4] = [0x16, 0x54, 0xAE, 0x6B];
const CLUSTER_ID: [u8; 4] = [0x1F, 0x43, 0xB6, 0x75];
const TIMECODE_SCALE_ID: [u8; 3] = [0x2A, 0xD7, 0xB1];
const DURATION_ID: [u8; 2] = [0x44, 0x89];

/// A `SeekHead` with two `Seek` entries, as both muxers write it ahead of `Info`.
fn seek_head() -> Vec<u8> {
    let mut entry = minimal(&[0x53, 0xAB], &INFO_ID);
    entry.extend(minimal(&[0x53, 0xAC], &[0x40]));
    let mut seeks = minimal(&[0x4D, 0xBB], &entry);
    seeks.extend(minimal(&[0x4D, 0xBB], &entry));
    minimal(&SEEK_HEAD_ID, &seeks)
}

/// How a muxer writes the Segment's children.
struct WebmShape {
    segment_size: Size,
    duration: Vec<u8>,
    seek_head_before_info: bool,
    cluster_bytes: usize,
}

fn webm_shaped(shape: &WebmShape) -> Vec<u8> {
    let mut info = minimal(&TIMECODE_SCALE_ID, &1_000_000_u32.to_be_bytes()[1..]);
    info.extend(minimal(&DURATION_ID, &shape.duration));
    info.extend(minimal(&[0x4D, 0x80], b"Lavf61.7.100"));
    info.extend(minimal(&[0x57, 0x41], b"Lavf61.7.100"));
    let mut segment = Vec::new();
    if shape.seek_head_before_info {
        segment.extend(seek_head());
        segment.extend(minimal(&VOID_ID, &[0; 40]));
    }
    segment.extend(minimal(&INFO_ID, &info));
    segment.extend(minimal(&TRACKS_ID, &[0; 64]));
    segment.extend(minimal(&CLUSTER_ID, &vec![0; shape.cluster_bytes]));
    let mut bytes = full_ebml_header();
    bytes.extend(ebml_element_styled(
        &SEGMENT_ID,
        &segment,
        shape.segment_size,
    ));
    bytes
}

/// What `ffmpeg -i in out.webm` writes: a `SeekHead`, a `Void`, then `Info` with a 64-bit float
/// `Duration`, in a Segment whose size is patched in at the end.
pub(crate) fn webm_ffmpeg_style(seconds: u32) -> Vec<u8> {
    webm_shaped(&WebmShape {
        segment_size: Size::Eight,
        duration: (f64::from(seconds) * 1000.0).to_be_bytes().to_vec(),
        seek_head_before_info: true,
        cluster_bytes: 4096,
    })
}

/// What libwebm writes (`aomenc`, `vpxenc`, Chrome's encoder): a 32-bit float `Duration`.
/// With `live_segment` the Segment's size is unknown, as a live muxer leaves it.
pub(crate) fn webm_libwebm_style(seconds: u32, live_segment: bool) -> Vec<u8> {
    webm_shaped(&WebmShape {
        segment_size: if live_segment {
            Size::Unknown
        } else {
            Size::Eight
        },
        duration: (f32::from(u16::try_from(seconds).expect("a test length fits 16 bits")) * 1000.0)
            .to_be_bytes()
            .to_vec(),
        seek_head_before_info: true,
        cluster_bytes: 4096,
    })
}

/// A webm with `Info` straight after the EBML header and no `SeekHead` or `Void`.
pub(crate) fn webm_info_first(seconds: u32) -> Vec<u8> {
    webm_shaped(&WebmShape {
        segment_size: Size::Eight,
        duration: (f64::from(seconds) * 1000.0).to_be_bytes().to_vec(),
        seek_head_before_info: false,
        cluster_bytes: 64,
    })
}

/// A webm whose `TimecodeScale` is written with `scale_bytes` bytes of zeros and a one.
pub(crate) fn webm_with_timecode_scale_bytes(scale_bytes: usize) -> Vec<u8> {
    let mut scale = vec![0_u8; scale_bytes];
    if let Some(last) = scale.last_mut() {
        *last = 1;
    }
    let mut info = minimal(&TIMECODE_SCALE_ID, &scale);
    info.extend(minimal(&DURATION_ID, &5000.0_f64.to_be_bytes()));
    let mut bytes = full_ebml_header();
    bytes.extend(ebml_element(&SEGMENT_ID, &minimal(&INFO_ID, &info)));
    bytes
}

/// A webm whose `Info` element claims `claimed_size` bytes, more than the file holds. The file
/// ends after `kept_info_bytes` bytes of its content.
pub(crate) fn webm_with_info_past_the_end(claimed_size: usize, kept_info_bytes: usize) -> Vec<u8> {
    let mut info = minimal(&TIMECODE_SCALE_ID, &1_000_000_u32.to_be_bytes()[1..]);
    info.extend(minimal(&DURATION_ID, &5000.0_f64.to_be_bytes()));
    info.truncate(kept_info_bytes);
    let mut segment = INFO_ID.to_vec();
    segment.extend(ebml_size(claimed_size, Size::Eight));
    segment.extend(info);
    let mut bytes = full_ebml_header();
    bytes.extend(ebml_element_styled(&SEGMENT_ID, &segment, Size::Unknown));
    bytes
}

/// How a webm's Segment Info is written.
pub(crate) struct WebmInfo {
    pub(crate) timecode_scale: Option<u64>,
    pub(crate) duration_ticks: Option<f64>,
}

/// A webm with the given document type and Segment Info, then a Cluster of `padding` bytes.
pub(crate) fn webm_with(doc_type: &str, info: &WebmInfo, padding: usize) -> Vec<u8> {
    let mut info_payload = Vec::new();
    if let Some(scale) = info.timecode_scale {
        info_payload.extend(ebml_element(&TIMECODE_SCALE_ID, &scale.to_be_bytes()));
    }
    if let Some(ticks) = info.duration_ticks {
        info_payload.extend(ebml_element(&DURATION_ID, &ticks.to_be_bytes()));
    }
    let mut segment = ebml_element(&INFO_ID, &info_payload);
    segment.extend(ebml_element(&CLUSTER_ID, &vec![0; padding]));
    let mut bytes = ebml_header(doc_type);
    bytes.extend(ebml_element(&SEGMENT_ID, &segment));
    bytes
}

/// The start of a webm whose EBML header holds a `DocType` that claims `claimed_bytes` bytes. The
/// payload is not included: a `SyntheticFile` supplies it as filler, standing for a huge DocType.
pub(crate) fn webm_doc_type_head(claimed_bytes: usize) -> Vec<u8> {
    let mut doc_type = vec![0x42, 0x82];
    doc_type.extend(ebml_size(claimed_bytes, Size::Eight));
    let header_payload_claim = claimed_bytes + doc_type.len();
    let mut bytes = vec![0x1A, 0x45, 0xDF, 0xA3];
    bytes.extend(ebml_size(header_payload_claim, Size::Eight));
    bytes.extend(doc_type);
    bytes
}

/// A valid webm that plays for `seconds` at the default timecode scale (1 ms a tick).
pub(crate) fn webm_of_seconds(seconds: u32) -> Vec<u8> {
    webm_with(
        "webm",
        &WebmInfo {
            timecode_scale: None,
            duration_ticks: Some(f64::from(seconds) * 1000.0),
        },
        16,
    )
}

/// A file of any length that holds only a few real bytes: a head, a tail and a filler byte
/// between them. It stands for a file of gigabytes without any of it in memory or on disk, and it
/// counts the bytes read from it, so a test can show that a reader touched only a few.
pub(crate) struct SyntheticFile {
    head: Vec<u8>,
    tail: Vec<u8>,
    length: u64,
    filler: u8,
    position: u64,
    pub(crate) bytes_read: u64,
}

impl SyntheticFile {
    pub(crate) fn new(head: Vec<u8>, tail: Vec<u8>, length: u64, filler: u8) -> Self {
        Self {
            head,
            tail,
            length,
            filler,
            position: 0,
            bytes_read: 0,
        }
    }

    fn byte_at(&self, position: u64) -> u8 {
        let head_length = u64::try_from(self.head.len()).expect("a head fits 64 bits");
        let tail_length = u64::try_from(self.tail.len()).expect("a tail fits 64 bits");
        if position < head_length {
            return self.head[usize::try_from(position).expect("a head index fits")];
        }
        let tail_start = self.length - tail_length;
        if position >= tail_start {
            return self.tail[usize::try_from(position - tail_start).expect("a tail index fits")];
        }
        self.filler
    }
}

impl Read for SyntheticFile {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let left = self.length.saturating_sub(self.position);
        let count = usize::try_from(left)
            .unwrap_or(usize::MAX)
            .min(buffer.len());
        for slot in buffer.iter_mut().take(count) {
            *slot = self.byte_at(self.position);
            self.position += 1;
        }
        self.bytes_read += u64::try_from(count).expect("a count fits 64 bits");
        Ok(count)
    }
}

impl Seek for SyntheticFile {
    fn seek(&mut self, target: SeekFrom) -> std::io::Result<u64> {
        let position = match target {
            SeekFrom::Start(offset) => Some(offset),
            SeekFrom::End(offset) => self.length.checked_add_signed(offset),
            SeekFrom::Current(offset) => self.position.checked_add_signed(offset),
        };
        let position = position
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "before start"))?;
        self.position = position;
        Ok(position)
    }
}

/// A one-second mp4 as a synthetic file of `length` bytes: `ftyp`, an `mdat` with a 64-bit size
/// covering the filler, then the `moov` at the very end (the layout `ffmpeg` writes by default).
pub(crate) fn synthetic_mp4_with_moov_last(length: u64) -> SyntheticFile {
    let moov = ffmpeg_moov(1000, 1000);
    let moov_length = u64::try_from(moov.len()).expect("a moov fits 64 bits");
    let mut head = ftyp(b"isom");
    let head_length = u64::try_from(head.len()).expect("a head fits 64 bits");
    head.extend_from_slice(&1_u32.to_be_bytes());
    head.extend_from_slice(b"mdat");
    head.extend_from_slice(&(length - head_length - moov_length).to_be_bytes());
    SyntheticFile::new(head, moov, length, 0)
}
