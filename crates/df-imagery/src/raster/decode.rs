//! Bounded decode of authorized bytes with digest and orientation checks.
//!
//! Sizes are probed from the container header before any pixel allocation.
//! EXIF orientation is normalized once, before recipe coordinates apply, and
//! all other metadata is left behind because only pixels cross the decode.

use std::io::Cursor;

use image::{ImageFormat, ImageReader, RgbaImage};
use sha2::{Digest, Sha256};

use super::RasterError;
use super::bounds::LiveBudget;
use super::geometry::{
    flip_horizontal, flip_vertical, rotate_180, rotate_270_counter_clockwise, rotate_90_clockwise,
};
use crate::request::{AssetDigest, SourceRef};

/// Resolves one pin through the injected bytes and verifies its digest and
/// input bound before the caller decodes.
pub(crate) fn resolve_verified(
    resolve: super::ResolveBytes<'_>,
    source: &SourceRef,
    budget: &LiveBudget,
) -> Result<Vec<u8>, RasterError> {
    let bytes = resolve(source)?;
    budget.check_input_len(bytes.len() as u64)?;
    verify_digest(&bytes, &source.digest)?;
    Ok(bytes)
}

fn verify_digest(bytes: &[u8], expected: &AssetDigest) -> Result<(), RasterError> {
    let actual: [u8; 32] = Sha256::digest(bytes).into();
    if &actual != expected.as_bytes() {
        return Err(RasterError::DigestMismatch);
    }
    Ok(())
}

/// Sniffs the container; only the formats the pinned encoder round-trips are
/// accepted, everything else is an explicit refusal rather than a guess.
pub(crate) fn sniff_format(bytes: &[u8]) -> Result<ImageFormat, RasterError> {
    match image::guess_format(bytes).map_err(|_| RasterError::UnsupportedFormat)? {
        format @ (ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP) => Ok(format),
        _ => Err(RasterError::UnsupportedFormat),
    }
}

/// Decodes authorized bytes to the canonical working RGBA raster. Header
/// dimensions bound the allocation; corrupt pixels fail without a default.
/// Embedded ICC profiles are refused instead of silently discarded because
/// no converter is pinned; sRGB inputs without a profile proceed.
pub(crate) fn decode_bounded(bytes: &[u8], budget: &LiveBudget) -> Result<RgbaImage, RasterError> {
    let format = sniff_format(bytes)?;
    if has_icc_profile(bytes, format) {
        return Err(RasterError::UnsupportedColorProfile);
    }
    let (width_px, height_px) = ImageReader::with_format(Cursor::new(bytes), format)
        .into_dimensions()
        .map_err(|_| RasterError::DecodeFailed)?;
    budget.check_pixels(width_px, height_px)?;
    let decoded =
        image::load_from_memory_with_format(bytes, format).map_err(|_| RasterError::DecodeFailed)?;
    let mut rgba = decoded.to_rgba8();
    if format == ImageFormat::Jpeg {
        rgba = apply_orientation(rgba, parse_exif_orientation(bytes));
    }
    Ok(rgba)
}

/// Detects an embedded ICC profile without converting it. PNG `iCCP`, JPEG
/// APP2 `ICC_PROFILE`, and WebP `ICCP` chunks all refuse; anything
/// unparseable is treated as no profile rather than a decode failure.
pub(crate) fn has_icc_profile(bytes: &[u8], format: ImageFormat) -> bool {
    match format {
        ImageFormat::Png => has_png_iccp(bytes),
        ImageFormat::Jpeg => has_jpeg_icc(bytes),
        ImageFormat::WebP => has_webp_iccp(bytes),
        _ => false,
    }
}

fn has_png_iccp(bytes: &[u8]) -> bool {
    const SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    if bytes.len() < 8 || &bytes[..8] != SIGNATURE {
        return false;
    }
    let mut pos = 8_usize;
    while pos + 8 <= bytes.len() {
        let len = u32::from_be_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]])
            as usize;
        let kind = &bytes[pos + 4..pos + 8];
        if kind == b"iCCP" {
            return true;
        }
        let Some(next) = pos.checked_add(8).and_then(|at| at.checked_add(len + 4)) else {
            return false;
        };
        if next <= pos || next > bytes.len() {
            return false;
        }
        if kind == b"IEND" {
            return false;
        }
        pos = next;
    }
    false
}

fn has_jpeg_icc(bytes: &[u8]) -> bool {
    if bytes.len() < 4 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
        return false;
    }
    let mut pos = 2_usize;
    while pos + 4 <= bytes.len() {
        if bytes[pos] != 0xFF {
            return false;
        }
        let marker = bytes[pos + 1];
        if marker == 0xD9 || marker == 0xDA {
            return false;
        }
        let segment_len = u16::from_be_bytes([bytes[pos + 2], bytes[pos + 3]]) as usize;
        if segment_len < 2 {
            return false;
        }
        let data_start = pos + 4;
        let Some(data_end) = pos.checked_add(2).and_then(|end| end.checked_add(segment_len)) else {
            return false;
        };
        if data_end > bytes.len() {
            return false;
        }
        if marker == 0xE2 && data_end - data_start >= 12 {
            let header = b"ICC_PROFILE\0";
            if &bytes[data_start..data_start + 12] == header {
                return true;
            }
        }
        pos = data_end;
    }
    false
}

fn has_webp_iccp(bytes: &[u8]) -> bool {
    if bytes.len() < 12
        || &bytes[..4] != b"RIFF"
        || &bytes[8..12] != b"WEBP"
    {
        return false;
    }
    let mut pos = 12_usize;
    while pos + 8 <= bytes.len() {
        let kind = &bytes[pos..pos + 4];
        let size = u32::from_le_bytes([bytes[pos + 4], bytes[pos + 5], bytes[pos + 6], bytes[pos + 7]])
            as usize;
        if kind == b"ICCP" {
            return true;
        }
        let padded = size + (size % 2);
        let Some(next) = pos.checked_add(8).and_then(|at| at.checked_add(padded)) else {
            return false;
        };
        if next <= pos || next > bytes.len() {
            return false;
        }
        pos = next;
    }
    false
}

/// Reads the EXIF orientation tag (1..=8) from JPEG APP1 bytes; anything
/// unparseable or absent is the identity orientation, never a failure.
pub(crate) fn parse_exif_orientation(bytes: &[u8]) -> u8 {
    if bytes.len() < 4 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
        return 1;
    }
    let mut pos = 2_usize;
    while pos + 4 <= bytes.len() {
        if bytes[pos] != 0xFF {
            return 1;
        }
        let marker = bytes[pos + 1];
        if marker == 0xD9 || marker == 0xDA {
            return 1;
        }
        let segment_len = u16::from_be_bytes([bytes[pos + 2], bytes[pos + 3]]) as usize;
        if segment_len < 2 {
            return 1;
        }
        let data_start = pos + 4;
        let data_end = pos.checked_add(2).and_then(|end| end.checked_add(segment_len));
        let Some(data_end) = data_end else {
            return 1;
        };
        if data_end > bytes.len() {
            return 1;
        }
        if marker == 0xE1 {
            let orientation = parse_exif_segment(&bytes[data_start..data_end]);
            if orientation != 1 {
                return orientation;
            }
        }
        pos = data_end;
    }
    1
}

fn parse_exif_segment(segment: &[u8]) -> u8 {
    const HEADER: &[u8] = b"Exif\0\0";
    if segment.len() < HEADER.len() + 8 || &segment[..HEADER.len()] != HEADER {
        return 1;
    }
    let tiff = &segment[HEADER.len()..];
    let little_endian = match &tiff[..2] {
        [0x49, 0x49] => true,
        [0x4D, 0x4D] => false,
        _ => return 1,
    };
    let read_u16 = |at: usize| -> Option<u16> {
        let pair: [u8; 2] = tiff.get(at..at + 2)?.try_into().ok()?;
        Some(if little_endian {
            u16::from_le_bytes(pair)
        } else {
            u16::from_be_bytes(pair)
        })
    };
    let read_u32 = |at: usize| -> Option<u32> {
        let quad: [u8; 4] = tiff.get(at..at + 4)?.try_into().ok()?;
        Some(if little_endian {
            u32::from_le_bytes(quad)
        } else {
            u32::from_be_bytes(quad)
        })
    };
    if read_u16(2) != Some(42) {
        return 1;
    }
    let ifd_offset = read_u32(4).unwrap_or(0) as usize;
    let entry_count = read_u16(ifd_offset).unwrap_or(0) as usize;
    for index in 0..entry_count {
        let entry = ifd_offset.checked_add(2).and_then(|at| at.checked_add(index * 12));
        let Some(entry) = entry else {
            return 1;
        };
        if read_u16(entry) != Some(0x0112) {
            continue;
        }
        // SHORT type with exactly one value: the value sits inline.
        if read_u16(entry + 2) != Some(3) || read_u32(entry + 4) != Some(1) {
            return 1;
        }
        let value = read_u16(entry + 8).unwrap_or(1);
        if (1..=8).contains(&value) {
            return value as u8;
        }
        return 1;
    }
    1
}

/// Applies an EXIF orientation to decoded pixels: 2 mirrors, 3 and 4 turn or
/// mirror, 5..=8 are the transposed forms. Orientation 1 is a no-op.
pub(crate) fn apply_orientation(image: RgbaImage, orientation: u8) -> RgbaImage {
    match orientation {
        2 => flip_horizontal(&image),
        3 => rotate_180(&image),
        4 => flip_vertical(&image),
        5 => flip_horizontal(&rotate_90_clockwise(&image)),
        6 => rotate_90_clockwise(&image),
        7 => flip_vertical(&rotate_90_clockwise(&image)),
        8 => rotate_270_counter_clockwise(&image),
        _ => image,
    }
}
