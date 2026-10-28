//! Private image-only codec: JPEG/WebP header facts for output checks.
//!
//! Relocated verbatim from the old provider adapter so `df-imagery` needs no
//! new dependency or second header parser. No network, no allocation beyond
//! the caller's slice.

/// Width and height of a complete WebP: the RIFF container size matches the
/// file and its first chunk is a lossy, lossless or extended image header.
pub(crate) fn webp_facts(bytes: &[u8]) -> Option<(u32, u32, bool)> {
    let u32_le = |at: usize| -> Option<u32> {
        Some(u32::from_le_bytes(
            bytes.get(at..at.checked_add(4)?)?.try_into().ok()?,
        ))
    };
    let u24_le = |at: usize| -> Option<u32> {
        let part = bytes.get(at..at.checked_add(3)?)?;
        Some(u32::from(part[0]) | (u32::from(part[1]) << 8) | (u32::from(part[2]) << 16))
    };
    if bytes.get(..4)? != b"RIFF"
        || bytes.get(8..12)? != b"WEBP"
        || usize::try_from(u32_le(4)?).ok()?.checked_add(8)? != bytes.len()
    {
        return None;
    }
    match bytes.get(12..16)? {
        b"VP8X" => {
            let alpha = bytes.get(20)? & 0x10 != 0;
            Some((u24_le(24)? + 1, u24_le(27)? + 1, alpha))
        }
        b"VP8L" => {
            if *bytes.get(20)? != 0x2f {
                return None;
            }
            let packed = u32_le(21)?;
            Some((
                (packed & 0x3fff) + 1,
                ((packed >> 14) & 0x3fff) + 1,
                (packed >> 28) & 1 == 1,
            ))
        }
        b"VP8 " => {
            if bytes.get(23..26)? != [0x9d, 0x01, 0x2a] {
                return None;
            }
            let width = u32::from(u16::from_le_bytes([*bytes.get(26)?, *bytes.get(27)?]) & 0x3fff);
            let height = u32::from(u16::from_le_bytes([*bytes.get(28)?, *bytes.get(29)?]) & 0x3fff);
            (width > 0 && height > 0).then_some((width, height, false))
        }
        _ => None,
    }
}

/// Width and height of a complete JPEG: starts with start-of-image, ends with
/// end-of-image and declares a frame.
pub(crate) fn jpeg_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.get(..2)? != [0xff, 0xd8] || bytes.get(bytes.len().checked_sub(2)?..)? != [0xff, 0xd9]
    {
        return None;
    }
    let mut position = 2usize;
    while position + 4 <= bytes.len() {
        if *bytes.get(position)? != 0xff {
            return None;
        }
        let marker = *bytes.get(position + 1)?;
        if marker == 0xff {
            position += 1;
            continue;
        }
        let length = usize::from(u16::from_be_bytes([
            *bytes.get(position + 2)?,
            *bytes.get(position + 3)?,
        ]));
        if matches!(marker, 0xc0..=0xc2) {
            let height = u16::from_be_bytes([*bytes.get(position + 5)?, *bytes.get(position + 6)?]);
            let width = u16::from_be_bytes([*bytes.get(position + 7)?, *bytes.get(position + 8)?]);
            return (width > 0 && height > 0).then_some((u32::from(width), u32::from(height)));
        }
        position = position.checked_add(2)?.checked_add(length)?;
    }
    None
}
