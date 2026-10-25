use crate::{DecodeBudget, DecodedImage, ResourceError};
use df_client::cache::{AssetCache, CacheError, CacheKey, CacheLease};
use std::cell::RefCell;
use std::rc::Rc;

/// Caller-selected presentation bounds, not measured browser/device capacity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageDecodeLimits {
    pub max_encoded_bytes: usize,
    pub max_dimension: u32,
    pub max_decoded_bytes: usize,
    pub max_work_bytes: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageDecodeError {
    InvalidLimits,
    MissingBytes,
    Lease(CacheError),
    Resource(ResourceError),
    OwnerBusy,
    WrongOwner,
    CorruptPng,
    UnsupportedPng,
    CorruptWebp,
    UnsupportedWebp,
    UnsupportedMime,
    MimeMismatch,
    AncillaryCapacity,
    DimensionCapacity,
    ByteCapacity,
    BrowserUnavailable,
    BrowserDecode,
    WrongDimensions,
    Cancelled,
    Deadline,
    OwnerDisposed,
}

/// Validated PNG/WebP presentation layout. Describes bounded work, never byte access.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageDecodePlan {
    pub width: u32,
    pub height: u32,
    pub budget: DecodeBudget,
}

/// Strict legacy RGBA8 PNG layout; prepared PNG/WebP callers use ImageDecodePlan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PngDecodePlan {
    pub width: u32,
    pub height: u32,
    pub budget: DecodeBudget,
}

impl ImageDecodePlan {
    fn png(self) -> PngDecodePlan {
        PngDecodePlan {
            width: self.width,
            height: self.height,
            budget: self.budget,
        }
    }
}

/// Supplied format/dimensions and a finite opaque ancillary-payload cap. These values
/// describe already permitted bytes; neither MIME nor embedded provenance grants access.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreparedImageMetadata<'a> {
    pub mime: &'a str,
    pub width: u32,
    pub height: u32,
    pub max_ancillary_bytes: usize,
}

fn read_u32(bytes: &[u8]) -> Result<u32, ImageDecodeError> {
    let array: [u8; 4] = bytes.try_into().map_err(|_| ImageDecodeError::CorruptPng)?;
    Ok(u32::from_be_bytes(array))
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & 0_u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}

fn validate_limits(bytes: &[u8], limits: ImageDecodeLimits) -> Result<(), ImageDecodeError> {
    if limits.max_encoded_bytes == 0
        || limits.max_dimension == 0
        || limits.max_decoded_bytes == 0
        || limits.max_work_bytes == 0
    {
        return Err(ImageDecodeError::InvalidLimits);
    }
    if bytes.len() > limits.max_encoded_bytes {
        return Err(ImageDecodeError::ByteCapacity);
    }
    Ok(())
}

fn image_plan(
    width: u32,
    height: u32,
    encoded_bytes: usize,
    channels: usize,
    limits: ImageDecodeLimits,
) -> Result<ImageDecodePlan, ImageDecodeError> {
    if width > limits.max_dimension || height > limits.max_dimension {
        return Err(ImageDecodeError::DimensionCapacity);
    }
    let decoded_bytes = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(ImageDecodeError::ByteCapacity)?;
    // Encoded cache input + Uint8Array + Blob, bitmap + scratch canvas + ImageData +
    // returned Rust RGBA + conservative copy, plus a decoded scanline/work allowance.
    // Codec-private allocator overhead remains subject to actual G08 device measurement.
    let scanline_bytes = (width as usize)
        .checked_mul(channels)
        .and_then(|row| row.checked_add(1))
        .and_then(|row| row.checked_mul(height as usize))
        .ok_or(ImageDecodeError::ByteCapacity)?;
    let work_bytes = encoded_bytes
        .checked_mul(3)
        .and_then(|encoded| {
            decoded_bytes
                .checked_mul(5)
                .and_then(|pixels| encoded.checked_add(pixels))
        })
        .and_then(|total| total.checked_add(scanline_bytes))
        .ok_or(ImageDecodeError::ByteCapacity)?;
    if decoded_bytes > limits.max_decoded_bytes || work_bytes > limits.max_work_bytes {
        return Err(ImageDecodeError::ByteCapacity);
    }
    Ok(ImageDecodePlan {
        width,
        height,
        budget: DecodeBudget {
            decoded_bytes,
            work_bytes,
        },
    })
}

/// Strict legacy RGBA8 PNG entry point: no metadata, palette or animation support.
/// CRC/framing validation never constitutes decoded success; the browser owns Deflate.
pub fn inspect_png(
    bytes: &[u8],
    limits: ImageDecodeLimits,
) -> Result<PngDecodePlan, ImageDecodeError> {
    inspect_png_bytes(bytes, limits, None).map(ImageDecodePlan::png)
}

fn inspect_png_bytes(
    bytes: &[u8],
    limits: ImageDecodeLimits,
    ancillary_cap: Option<usize>,
) -> Result<ImageDecodePlan, ImageDecodeError> {
    if let Some(cap) = ancillary_cap {
        return inspect_prepared_png_bytes(bytes, limits, cap);
    }
    validate_limits(bytes, limits)?;
    if bytes.get(..8) != Some(b"\x89PNG\r\n\x1a\n") {
        return Err(ImageDecodeError::CorruptPng);
    }
    let mut cursor = 8_usize;
    let mut dimensions = None;
    let mut channels = 4;
    let mut has_data = false;
    let mut has_cabx = false;
    loop {
        let header_end = cursor.checked_add(8).ok_or(ImageDecodeError::CorruptPng)?;
        let header = bytes
            .get(cursor..header_end)
            .ok_or(ImageDecodeError::CorruptPng)?;
        let length = read_u32(header.get(..4).ok_or(ImageDecodeError::CorruptPng)?)? as usize;
        let kind = header.get(4..).ok_or(ImageDecodeError::CorruptPng)?;
        let data_end = header_end
            .checked_add(length)
            .ok_or(ImageDecodeError::CorruptPng)?;
        let end = data_end
            .checked_add(4)
            .ok_or(ImageDecodeError::CorruptPng)?;
        let data = bytes
            .get(header_end..data_end)
            .ok_or(ImageDecodeError::CorruptPng)?;
        let expected = read_u32(
            bytes
                .get(data_end..end)
                .ok_or(ImageDecodeError::CorruptPng)?,
        )?;
        if crc32(
            bytes
                .get(cursor + 4..data_end)
                .ok_or(ImageDecodeError::CorruptPng)?,
        ) != expected
        {
            return Err(ImageDecodeError::CorruptPng);
        }
        match kind {
            b"IHDR" if cursor == 8 && dimensions.is_none() && length == 13 => {
                let width = read_u32(data.get(..4).ok_or(ImageDecodeError::CorruptPng)?)?;
                let height = read_u32(data.get(4..8).ok_or(ImageDecodeError::CorruptPng)?)?;
                if width == 0 || height == 0 {
                    return Err(ImageDecodeError::CorruptPng);
                }
                if width > limits.max_dimension || height > limits.max_dimension {
                    return Err(ImageDecodeError::DimensionCapacity);
                }
                channels = match data.get(8..) {
                    Some([8, 6, 0, 0, 0]) => 4,
                    Some([8, 2, 0, 0, 0]) if ancillary_cap.is_some() => 3,
                    _ => return Err(ImageDecodeError::UnsupportedPng),
                };
                dimensions = Some((width, height));
            }
            b"caBX" if dimensions.is_some() && !has_data && !has_cabx => {
                let cap = ancillary_cap.ok_or(ImageDecodeError::UnsupportedPng)?;
                if length > cap {
                    return Err(ImageDecodeError::AncillaryCapacity);
                }
                // Opaque payload is CRC checked and bounded, never parsed as rights or authority.
                has_cabx = true;
            }
            b"IDAT" if dimensions.is_some() && length != 0 => has_data = true,
            b"IEND" if dimensions.is_some() && has_data && length == 0 && end == bytes.len() => {
                break;
            }
            b"IHDR" | b"IDAT" | b"IEND" => return Err(ImageDecodeError::CorruptPng),
            _ => return Err(ImageDecodeError::UnsupportedPng),
        }
        cursor = end;
    }
    let (width, height) = dimensions.ok_or(ImageDecodeError::CorruptPng)?;
    image_plan(width, height, bytes.len(), channels, limits)
}

// The campaign's existing PNG scope includes gray/indexed/RGB/alpha, 16-bit and
// Adam7 input. Browser ImageBitmap converts these to the same bounded RGBA8 output.
// Native checks validate framing/layout only; CRC/header success is never decode success.
fn inspect_prepared_png_bytes(
    bytes: &[u8],
    limits: ImageDecodeLimits,
    ancillary_cap: usize,
) -> Result<ImageDecodePlan, ImageDecodeError> {
    validate_limits(bytes, limits)?;
    if bytes.get(..8) != Some(b"\x89PNG\r\n\x1a\n") {
        return Err(ImageDecodeError::CorruptPng);
    }
    let mut cursor = 8_usize;
    let mut layout = None;
    let mut palette = None;
    let mut transparency = false;
    let mut has_cabx = false;
    let mut has_data = false;
    let mut data_ended = false;
    let mut ancillary = 0_usize;
    loop {
        let header_end = cursor.checked_add(8).ok_or(ImageDecodeError::CorruptPng)?;
        let header = bytes
            .get(cursor..header_end)
            .ok_or(ImageDecodeError::CorruptPng)?;
        let len = read_u32(header.get(..4).ok_or(ImageDecodeError::CorruptPng)?)? as usize;
        let kind = header.get(4..).ok_or(ImageDecodeError::CorruptPng)?;
        if kind.iter().any(|value| !value.is_ascii_alphabetic())
            || kind.get(2).is_some_and(u8::is_ascii_lowercase)
        {
            return Err(ImageDecodeError::CorruptPng);
        }
        let data_end = header_end
            .checked_add(len)
            .ok_or(ImageDecodeError::CorruptPng)?;
        let end = data_end
            .checked_add(4)
            .ok_or(ImageDecodeError::CorruptPng)?;
        let data = bytes
            .get(header_end..data_end)
            .ok_or(ImageDecodeError::CorruptPng)?;
        let checksum = read_u32(
            bytes
                .get(data_end..end)
                .ok_or(ImageDecodeError::CorruptPng)?,
        )?;
        if crc32(
            bytes
                .get(cursor + 4..data_end)
                .ok_or(ImageDecodeError::CorruptPng)?,
        ) != checksum
        {
            return Err(ImageDecodeError::CorruptPng);
        }
        match kind {
            b"IHDR" if cursor == 8 && len == 13 => {
                let width = read_u32(data.get(..4).ok_or(ImageDecodeError::CorruptPng)?)?;
                let height = read_u32(data.get(4..8).ok_or(ImageDecodeError::CorruptPng)?)?;
                let tail = data.get(8..).ok_or(ImageDecodeError::CorruptPng)?;
                let (depth, color, interlace) = match tail {
                    [depth, color, 0, 0, interlace] if *interlace <= 1 => {
                        (*depth, *color, *interlace)
                    }
                    _ => return Err(ImageDecodeError::UnsupportedPng),
                };
                let channels = match (color, depth) {
                    (0, 1 | 2 | 4 | 8) | (3, 1 | 2 | 4 | 8) => 1,
                    (0, 16) | (4, 8) => 2,
                    (2, 8) => 3,
                    (6, 8) | (4, 16) => 4,
                    (2, 16) => 6,
                    (6, 16) => 8,
                    _ => return Err(ImageDecodeError::UnsupportedPng),
                };
                if width == 0 || height == 0 {
                    return Err(ImageDecodeError::CorruptPng);
                }
                layout = Some((width, height, depth, color, interlace, channels));
            }
            b"PLTE" if layout.is_some() && !has_data && palette.is_none() => {
                let (_, _, depth, color, _, _) = layout.ok_or(ImageDecodeError::CorruptPng)?;
                if matches!(color, 0 | 4)
                    || len == 0
                    || !len.is_multiple_of(3)
                    || len > 768
                    || (color == 3 && len / 3 > 1_usize << depth)
                {
                    return Err(ImageDecodeError::CorruptPng);
                }
                palette = Some(len / 3);
            }
            b"tRNS" if layout.is_some() && !has_data && !transparency => {
                let (_, _, _, color, _, _) = layout.ok_or(ImageDecodeError::CorruptPng)?;
                let valid = match color {
                    0 => len == 2,
                    2 => len == 6,
                    3 => palette.is_some_and(|colors| len > 0 && len <= colors),
                    _ => false,
                };
                if !valid {
                    return Err(ImageDecodeError::CorruptPng);
                }
                transparency = true;
            }
            b"IDAT" if layout.is_some() && !data_ended => {
                if layout.is_some_and(|(_, _, _, color, _, _)| color == 3) && palette.is_none() {
                    return Err(ImageDecodeError::CorruptPng);
                }
                has_data |= len > 0;
            }
            b"IEND" if layout.is_some() && has_data && len == 0 && end == bytes.len() => break,
            b"IHDR" | b"PLTE" | b"IDAT" | b"IEND" | b"tRNS" => {
                return Err(ImageDecodeError::CorruptPng);
            }
            // Compressed text/profiles and APNG need separate decoded metadata/frame budgets.
            b"iCCP" | b"zTXt" | b"acTL" | b"fcTL" | b"fdAT" => {
                return Err(ImageDecodeError::UnsupportedPng);
            }
            b"iTXt"
                if data
                    .iter()
                    .position(|byte| *byte == 0)
                    .and_then(|separator| data.get(separator + 1))
                    .is_some_and(|flag| *flag == 0) => {}
            b"caBX" if layout.is_some() && !has_data && !has_cabx => {
                has_cabx = true;
            }
            b"tEXt" | b"gAMA" | b"cHRM" | b"sRGB" | b"bKGD" | b"pHYs" | b"sBIT" | b"tIME"
                if layout.is_some() => {}
            _ => return Err(ImageDecodeError::UnsupportedPng),
        }
        if kind != b"IDAT" && has_data {
            data_ended = true;
        }
        if kind.first().is_some_and(u8::is_ascii_lowercase) {
            ancillary = ancillary
                .checked_add(len)
                .ok_or(ImageDecodeError::AncillaryCapacity)?;
            if ancillary > ancillary_cap {
                return Err(ImageDecodeError::AncillaryCapacity);
            }
        }
        cursor = end;
    }
    let (width, height, _, _, interlace, channels) = layout.ok_or(ImageDecodeError::CorruptPng)?;
    let mut plan = image_plan(width, height, bytes.len(), channels, limits)?;
    if interlace == 1 {
        // At most seven pass scanline filter bytes per source row; image_plan already
        // reserves one. Source sample bytes stay bounded by channels*width*height.
        plan.budget.work_bytes = plan
            .budget
            .work_bytes
            .checked_add(
                (height as usize)
                    .checked_mul(6)
                    .ok_or(ImageDecodeError::ByteCapacity)?,
            )
            .ok_or(ImageDecodeError::ByteCapacity)?;
        if plan.budget.work_bytes > limits.max_work_bytes {
            return Err(ImageDecodeError::ByteCapacity);
        }
    }
    Ok(plan)
}

fn read_webp_u32(bytes: &[u8]) -> Result<u32, ImageDecodeError> {
    let array: [u8; 4] = bytes
        .try_into()
        .map_err(|_| ImageDecodeError::CorruptWebp)?;
    Ok(u32::from_le_bytes(array))
}

fn inspect_webp(
    bytes: &[u8],
    limits: ImageDecodeLimits,
) -> Result<ImageDecodePlan, ImageDecodeError> {
    validate_limits(bytes, limits)?;
    if bytes.get(..4) != Some(b"RIFF") || bytes.get(8..12) != Some(b"WEBP") {
        return Err(ImageDecodeError::CorruptWebp);
    }
    let total_bytes = (read_webp_u32(bytes.get(4..8).ok_or(ImageDecodeError::CorruptWebp)?)?
        as usize)
        .checked_add(8)
        .ok_or(ImageDecodeError::CorruptWebp)?;
    if total_bytes != bytes.len() {
        return Err(ImageDecodeError::CorruptWebp);
    }
    if bytes.get(12..16) != Some(b"VP8 ") {
        return Err(ImageDecodeError::UnsupportedWebp);
    }
    let length = read_webp_u32(bytes.get(16..20).ok_or(ImageDecodeError::CorruptWebp)?)? as usize;
    let data_end = 20_usize
        .checked_add(length)
        .ok_or(ImageDecodeError::CorruptWebp)?;
    let padded_end = data_end
        .checked_add(length & 1)
        .ok_or(ImageDecodeError::CorruptWebp)?;
    if padded_end != bytes.len() || (length & 1 != 0 && bytes.get(data_end) != Some(&0)) {
        return Err(ImageDecodeError::CorruptWebp);
    }
    let frame = bytes
        .get(20..data_end)
        .ok_or(ImageDecodeError::CorruptWebp)?;
    let header = frame.get(..10).ok_or(ImageDecodeError::CorruptWebp)?;
    let tag_bytes: [u8; 3] = header
        .get(..3)
        .ok_or(ImageDecodeError::CorruptWebp)?
        .try_into()
        .map_err(|_| ImageDecodeError::CorruptWebp)?;
    let tag = u32::from_le_bytes([tag_bytes[0], tag_bytes[1], tag_bytes[2], 0]);
    if tag & 1 != 0 || (tag >> 1) & 7 > 3 || tag & 16 == 0 {
        return Err(ImageDecodeError::UnsupportedWebp);
    }
    if header.get(3..6) != Some(&[0x9d, 0x01, 0x2a]) {
        return Err(ImageDecodeError::CorruptWebp);
    }
    let width_bytes: [u8; 2] = header
        .get(6..8)
        .ok_or(ImageDecodeError::CorruptWebp)?
        .try_into()
        .map_err(|_| ImageDecodeError::CorruptWebp)?;
    let height_bytes: [u8; 2] = header
        .get(8..10)
        .ok_or(ImageDecodeError::CorruptWebp)?
        .try_into()
        .map_err(|_| ImageDecodeError::CorruptWebp)?;
    let width = u16::from_le_bytes(width_bytes);
    let height = u16::from_le_bytes(height_bytes);
    if width & 0xc000 != 0 || height & 0xc000 != 0 {
        return Err(ImageDecodeError::UnsupportedWebp);
    }
    if width == 0 || height == 0 || tag >> 5 == 0 || (tag >> 5) as usize > length - 10 {
        return Err(ImageDecodeError::CorruptWebp);
    }
    // One actual prepared lossy keyframe only: VP8X, lossless, alpha, metadata, animations
    // and additional RIFF chunks are explicitly unsupported rather than silently decoded.
    image_plan(u32::from(width), u32::from(height), bytes.len(), 4, limits)
}

/// Checks MIME/container agreement, exact declared dimensions, finite payload limits and
/// framing before any browser allocation. Complete/hash/current lease remain AssetCache work.
pub fn inspect_prepared_image(
    bytes: &[u8],
    metadata: PreparedImageMetadata<'_>,
    limits: ImageDecodeLimits,
) -> Result<ImageDecodePlan, ImageDecodeError> {
    let plan = match metadata.mime {
        "image/png" => {
            if bytes.get(..4) == Some(b"RIFF") {
                return Err(ImageDecodeError::MimeMismatch);
            }
            inspect_png_bytes(bytes, limits, Some(metadata.max_ancillary_bytes))?
        }
        "image/webp" => {
            if bytes.get(..8) == Some(b"\x89PNG\r\n\x1a\n") {
                return Err(ImageDecodeError::MimeMismatch);
            }
            inspect_webp(bytes, limits)?
        }
        _ => return Err(ImageDecodeError::UnsupportedMime),
    };
    if (plan.width, plan.height) != (metadata.width, metadata.height) {
        return Err(ImageDecodeError::WrongDimensions);
    }
    Ok(plan)
}

/// Owns one actual canonical lease for a validated prepared PNG/WebP without copying
/// input bytes. Callers admit plan.budget before browser allocation. Metadata grants no access.
pub struct VerifiedImage {
    cache: Rc<RefCell<AssetCache>>,
    lease: CacheLease,
    plan: ImageDecodePlan,
    mime: &'static str,
}

impl VerifiedImage {
    /// Acquires the canonical lease to inspect approved bytes. Refusal drops that temporary
    /// lease before returning and never reserves codec work; successful input retains one lease.
    pub fn acquire_prepared(
        cache: Rc<RefCell<AssetCache>>,
        key: &CacheKey,
        metadata: PreparedImageMetadata<'_>,
        limits: ImageDecodeLimits,
    ) -> Result<Self, ImageDecodeError> {
        Self::acquire_input(cache, key, limits, Some(metadata))
    }

    fn acquire_input(
        cache: Rc<RefCell<AssetCache>>,
        key: &CacheKey,
        limits: ImageDecodeLimits,
        metadata: Option<PreparedImageMetadata<'_>>,
    ) -> Result<Self, ImageDecodeError> {
        let lease = cache
            .try_borrow_mut()
            .map_err(|_| ImageDecodeError::OwnerBusy)?
            .acquire(key)
            .map_err(ImageDecodeError::Lease)?
            .ok_or(ImageDecodeError::MissingBytes)?;
        let (plan, mime) = {
            let owner = cache
                .try_borrow()
                .map_err(|_| ImageDecodeError::OwnerBusy)?;
            let bytes = owner.lease_bytes(&lease).map_err(ImageDecodeError::Lease)?;
            match metadata {
                Some(metadata) => {
                    let plan = inspect_prepared_image(bytes, metadata, limits)?;
                    let mime = match metadata.mime {
                        "image/png" => "image/png",
                        "image/webp" => "image/webp",
                        _ => return Err(ImageDecodeError::UnsupportedMime),
                    };
                    (plan, mime)
                }
                None => (inspect_png_bytes(bytes, limits, None)?, "image/png"),
            }
        };
        Ok(Self {
            cache,
            lease,
            plan,
            mime,
        })
    }

    pub fn mime(&self) -> &'static str {
        self.mime
    }

    pub fn plan(&self) -> ImageDecodePlan {
        self.plan
    }
    pub fn key(&self) -> &CacheKey {
        self.lease.key()
    }

    pub(crate) fn cache_lease(&self) -> &CacheLease {
        &self.lease
    }

    pub(crate) fn belongs_to(&self, cache: &Rc<RefCell<AssetCache>>) -> bool {
        Rc::ptr_eq(&self.cache, cache)
    }

    pub fn validate_current(&self) -> Result<(), ImageDecodeError> {
        self.cache
            .try_borrow()
            .map_err(|_| ImageDecodeError::OwnerBusy)?
            .lease_bytes(&self.lease)
            .map_err(ImageDecodeError::Lease)?;
        Ok(())
    }

    /// Terminal codec boundary. Native tests supply controlled RGBA here; production
    /// browser callers reach it only after the actual ImageBitmap/canvas decode.
    pub fn finish_rgba(
        self,
        width: u32,
        height: u32,
        pixels: Box<[u8]>,
    ) -> Result<DecodedImage<CacheKey>, ImageDecodeError> {
        self.validate_current()?;
        if (width, height) != (self.plan.width, self.plan.height) {
            return Err(ImageDecodeError::WrongDimensions);
        }
        if pixels.len() != self.plan.budget.decoded_bytes {
            return Err(ImageDecodeError::ByteCapacity);
        }
        let key = self.lease.key().clone();
        let lease = self.lease.clone();
        let binding = Rc::new(self);
        let current = binding.clone();
        DecodedImage::new(
            key,
            width,
            height,
            pixels,
            move |expected| current.key() == expected && current.validate_current().is_ok(),
            move || drop(binding),
        )
        .map(|image| image.bind_cache_lease(lease))
        .map_err(ImageDecodeError::Resource)
        // Provenance and both guards share the exact existing lease identity. Dropping
        // the last handle releases that slot under AssetCache's canonical weak lifecycle.
    }
}

/// Strict legacy carrier. Its private inner image is acquired only as RGBA8 PNG;
/// there is no prepared-format constructor or neutral-to-PNG public conversion.
pub struct VerifiedPng(VerifiedImage);

impl VerifiedPng {
    pub fn acquire(
        cache: Rc<RefCell<AssetCache>>,
        key: &CacheKey,
        limits: ImageDecodeLimits,
    ) -> Result<Self, ImageDecodeError> {
        VerifiedImage::acquire_input(cache, key, limits, None).map(Self)
    }

    pub fn plan(&self) -> PngDecodePlan {
        self.0.plan().png()
    }
    pub fn mime(&self) -> &'static str {
        self.0.mime()
    }
    pub fn key(&self) -> &CacheKey {
        self.0.key()
    }
    pub fn validate_current(&self) -> Result<(), ImageDecodeError> {
        self.0.validate_current()
    }

    pub(crate) fn image(&self) -> &VerifiedImage {
        &self.0
    }

    /// Uses the same terminal boundary and exact canonical lease as the neutral codec.
    pub fn finish_rgba(
        self,
        width: u32,
        height: u32,
        pixels: Box<[u8]>,
    ) -> Result<DecodedImage<CacheKey>, ImageDecodeError> {
        self.0.finish_rgba(width, height, pixels)
    }
}

#[cfg(target_arch = "wasm32")]
mod browser {
    use super::*;
    use crate::{
        BrowserResourceScene, DecodeToken, ImageSurfaceError, ResourceLifecycle,
        ResourceLifecycleError,
    };
    use js_sys::{Array, Uint8Array};
    use std::cell::Cell;
    use std::future::poll_fn;
    use std::task::{Poll, Waker};
    use wasm_bindgen::{JsCast, closure::Closure};
    use wasm_bindgen_futures::{JsFuture, spawn_local};
    use web_sys::{
        Blob, BlobPropertyBag, CanvasRenderingContext2d, ColorSpaceConversion, HtmlCanvasElement,
        ImageBitmap, ImageBitmapOptions, PremultiplyAlpha, Window,
    };

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum BrowserDecodeStatus {
        Running,
        Draining(ImageDecodeError),
        Terminal(Result<bool, ImageDecodeError>),
    }

    fn fence_deadline(
        stop: &Cell<Option<ImageDecodeError>>,
        status: &Cell<BrowserDecodeStatus>,
    ) -> bool {
        if matches!(status.get(), BrowserDecodeStatus::Terminal(_)) || stop.get().is_some() {
            return false;
        }
        stop.set(Some(ImageDecodeError::Deadline));
        status.set(BrowserDecodeStatus::Draining(ImageDecodeError::Deadline));
        true
    }

    struct Deadline {
        window: Window,
        id: i32,
        _callback: Closure<dyn FnMut()>,
    }
    impl Drop for Deadline {
        fn drop(&mut self) {
            self.window.clear_timeout_with_handle(self.id);
        }
    }
    struct Prepared {
        blob: Blob,
        promise: js_sys::Promise,
    }
    struct Bitmap(ImageBitmap);
    impl Drop for Bitmap {
        fn drop(&mut self) {
            self.0.close();
        }
    }
    struct Scratch(HtmlCanvasElement);
    impl Drop for Scratch {
        fn drop(&mut self) {
            self.0.set_width(0);
            self.0.set_height(0);
            self.0.remove();
        }
    }

    #[derive(Clone)]
    enum DecodeOwner {
        Scene(Rc<RefCell<BrowserResourceScene<CacheKey>>>),
        Lifecycle(Rc<RefCell<ResourceLifecycle>>),
    }

    impl DecodeOwner {
        fn begin(
            &self,
            input: &VerifiedImage,
            abort: impl FnOnce() + 'static,
        ) -> Result<DecodeToken<CacheKey>, ImageDecodeError> {
            match self {
                Self::Scene(scene) => scene
                    .try_borrow_mut()
                    .map_err(|_| ImageDecodeError::OwnerBusy)?
                    .begin(input.key(), input.plan().budget, abort)
                    .map_err(ImageDecodeError::Resource),
                Self::Lifecycle(lifecycle) => lifecycle
                    .try_borrow_mut()
                    .map_err(|_| ImageDecodeError::OwnerBusy)?
                    .begin_image(input, abort)
                    .map_err(lifecycle_error),
            }
        }

        fn finish_failed(&self, token: &DecodeToken<CacheKey>) -> Result<(), ImageDecodeError> {
            match self {
                Self::Scene(scene) => scene
                    .try_borrow_mut()
                    .map_err(|_| ImageDecodeError::OwnerBusy)?
                    .finish_failed(token)
                    .map(|_| ())
                    .map_err(ImageDecodeError::Resource),
                Self::Lifecycle(lifecycle) => lifecycle
                    .try_borrow_mut()
                    .map_err(|_| ImageDecodeError::OwnerBusy)?
                    .finish_failed(token)
                    .map(|_| ())
                    .map_err(ImageDecodeError::Resource),
            }
        }

        fn cancel(&self, token: &DecodeToken<CacheKey>) -> Result<(), ImageDecodeError> {
            match self {
                Self::Scene(scene) => scene
                    .try_borrow_mut()
                    .map_err(|_| ImageDecodeError::OwnerBusy)?
                    .cancel(token)
                    .map_err(ImageDecodeError::Resource),
                Self::Lifecycle(lifecycle) => lifecycle
                    .try_borrow_mut()
                    .map_err(|_| ImageDecodeError::OwnerBusy)?
                    .cancel(token)
                    .map_err(ImageDecodeError::Resource),
            }
        }

        fn complete_and_present(
            &self,
            token: &DecodeToken<CacheKey>,
            image: DecodedImage<CacheKey>,
        ) -> Result<bool, ImageDecodeError> {
            match self {
                Self::Scene(scene) => {
                    let mut scene = scene
                        .try_borrow_mut()
                        .map_err(|_| ImageDecodeError::OwnerBusy)?;
                    scene
                        .complete(token, image)
                        .map_err(ImageDecodeError::Resource)?;
                    scene.present(token.key()).map_err(surface_error)
                }
                Self::Lifecycle(lifecycle) => {
                    let mut lifecycle = lifecycle
                        .try_borrow_mut()
                        .map_err(|_| ImageDecodeError::OwnerBusy)?;
                    lifecycle
                        .complete(token, image)
                        .map_err(ImageDecodeError::Resource)?;
                    lifecycle.present(token.key()).map_err(surface_error)
                }
            }
        }
    }

    fn surface_error(error: ImageSurfaceError) -> ImageDecodeError {
        match error {
            ImageSurfaceError::Resource(error) => ImageDecodeError::Resource(error),
            ImageSurfaceError::SurfaceCapacity => ImageDecodeError::ByteCapacity,
            ImageSurfaceError::Scene(_) | ImageSurfaceError::Browser => {
                ImageDecodeError::BrowserDecode
            }
        }
    }

    fn lifecycle_error(error: ResourceLifecycleError) -> ImageDecodeError {
        match error {
            ResourceLifecycleError::Cache(error) => ImageDecodeError::Lease(error),
            ResourceLifecycleError::Decode(error) => error,
            ResourceLifecycleError::Resource(error) => ImageDecodeError::Resource(error),
            ResourceLifecycleError::Scene(_) | ResourceLifecycleError::UnsupportedTarget => {
                ImageDecodeError::BrowserDecode
            }
            ResourceLifecycleError::Surface(error) => surface_error(error),
        }
    }

    /// One named operation owned by the mounted scope. Dropping/cancelling this handle
    /// forbids publication immediately; the retained runner owns its cache/scene/input,
    /// timer and promise until actual terminal completion, then exposes a fixed-size result.
    /// A nonsettling browser promise remains counted, never admits uncounted replacement work.
    /// Scope callers must release synchronous scene/cache borrows before awaiting completion.
    pub struct BrowserImageDecode {
        owner: DecodeOwner,
        token: DecodeToken<CacheKey>,
        stop: Rc<Cell<Option<ImageDecodeError>>>,
        status: Rc<Cell<BrowserDecodeStatus>>,
        waiter: Rc<RefCell<Option<Waker>>>,
    }

    impl BrowserImageDecode {
        pub fn start(
            scene: Rc<RefCell<BrowserResourceScene<CacheKey>>>,
            cache: Rc<RefCell<AssetCache>>,
            key: &CacheKey,
            limits: ImageDecodeLimits,
            deadline_ms: i32,
        ) -> Result<Self, ImageDecodeError> {
            if deadline_ms <= 0 {
                return Err(ImageDecodeError::InvalidLimits);
            }
            let window = web_sys::window().ok_or(ImageDecodeError::BrowserUnavailable)?;
            let scope = cache
                .try_borrow()
                .map_err(|_| ImageDecodeError::OwnerBusy)?
                .scope();
            let owner = scene
                .try_borrow()
                .map_err(|_| ImageDecodeError::OwnerBusy)?
                .owner();
            if (scope.session, scope.run, scope.binding) != owner {
                return Err(ImageDecodeError::WrongOwner);
            }
            let input = VerifiedPng::acquire(cache, key, limits)?.0;
            Self::start_input(DecodeOwner::Scene(scene), input, window, deadline_ms)
        }

        /// Starts the same real codec through the closed lifecycle's canonical byte/scene
        /// owner. Input acquisition, reservation and all terminal bookkeeping stay with
        /// that owner; its private cache and scene handles never escape through this API.
        pub fn start_owned(
            lifecycle: Rc<RefCell<ResourceLifecycle>>,
            key: &CacheKey,
            limits: ImageDecodeLimits,
            deadline_ms: i32,
        ) -> Result<Self, ImageDecodeError> {
            if deadline_ms <= 0 {
                return Err(ImageDecodeError::InvalidLimits);
            }
            let window = web_sys::window().ok_or(ImageDecodeError::BrowserUnavailable)?;
            let input = {
                let owner = lifecycle
                    .try_borrow()
                    .map_err(|_| ImageDecodeError::OwnerBusy)?;
                owner.prepare_png(key, limits)?.0
            };
            Self::start_input(
                DecodeOwner::Lifecycle(lifecycle),
                input,
                window,
                deadline_ms,
            )
        }

        /// Same owned codec for a caller-described, already permitted prepared PNG/WebP.
        pub fn start_owned_prepared(
            lifecycle: Rc<RefCell<ResourceLifecycle>>,
            key: &CacheKey,
            metadata: PreparedImageMetadata<'_>,
            limits: ImageDecodeLimits,
            deadline_ms: i32,
        ) -> Result<Self, ImageDecodeError> {
            if deadline_ms <= 0 {
                return Err(ImageDecodeError::InvalidLimits);
            }
            let window = web_sys::window().ok_or(ImageDecodeError::BrowserUnavailable)?;
            let input = {
                let owner = lifecycle
                    .try_borrow()
                    .map_err(|_| ImageDecodeError::OwnerBusy)?;
                owner.prepare_image(key, metadata, limits)?
            };
            Self::start_input(
                DecodeOwner::Lifecycle(lifecycle),
                input,
                window,
                deadline_ms,
            )
        }

        fn start_input(
            owner: DecodeOwner,
            input: VerifiedImage,
            window: Window,
            deadline_ms: i32,
        ) -> Result<Self, ImageDecodeError> {
            let stop = Rc::new(Cell::new(None));
            let aborted = stop.clone();
            let token = owner.begin(&input, move || {
                if aborted.get().is_none() {
                    aborted.set(Some(ImageDecodeError::Cancelled));
                }
            })?;
            let status = Rc::new(Cell::new(BrowserDecodeStatus::Running));
            let timed_stop = stop.clone();
            let timed_status = status.clone();
            let callback = Closure::<dyn FnMut()>::new(move || {
                fence_deadline(&timed_stop, &timed_status);
            });
            let timer = window.set_timeout_with_callback_and_timeout_and_arguments_0(
                callback.as_ref().unchecked_ref(),
                deadline_ms,
            );
            let id = match timer {
                Ok(id) => id,
                Err(_) => {
                    owner.finish_failed(&token)?;
                    return Err(ImageDecodeError::BrowserUnavailable);
                }
            };
            let deadline = Deadline {
                window,
                id,
                _callback: callback,
            };
            // The actual browser promise starts inside successful admission, before returning
            // the handle. Cancel/revoke/dispose can therefore race real running codec work.
            let prepared = match prepare(&input) {
                Ok(prepared) => prepared,
                Err(error) => {
                    drop(deadline);
                    owner.finish_failed(&token)?;
                    return Err(error);
                }
            };
            let waiter = Rc::new(RefCell::new(None::<Waker>));
            let handle = Self {
                owner: owner.clone(),
                token: token.clone(),
                stop: stop.clone(),
                status: status.clone(),
                waiter: waiter.clone(),
            };
            spawn_local(async move {
                let decoded = decode(input, prepared, &stop).await;
                // Bitmap/canvas/Blob/input allocations reach terminal before reservation release.
                drop(deadline);
                let result = settle(&owner, &token, decoded, stop.get());
                status.set(BrowserDecodeStatus::Terminal(result));
                if let Some(waker) = waiter.borrow_mut().take() {
                    waker.wake();
                }
            });
            Ok(handle)
        }

        pub fn status(&self) -> BrowserDecodeStatus {
            if let Some(reason) = self.stop.get()
                && !matches!(self.status.get(), BrowserDecodeStatus::Terminal(_))
            {
                return BrowserDecodeStatus::Draining(reason);
            }
            self.status.get()
        }

        /// Controlled fixture delivery of the same fence used by the owned browser timer.
        /// It never completes, rejects or substitutes the actual running codec promise.
        /// Ordinary production builds omit this entry point.
        #[cfg(feature = "image-decode-fixture")]
        pub fn fixture_fire_deadline(&mut self) -> bool {
            fence_deadline(&self.stop, &self.status)
        }

        pub fn cancel(&mut self) -> Result<(), ImageDecodeError> {
            if matches!(self.status.get(), BrowserDecodeStatus::Terminal(_)) {
                return Ok(());
            }
            self.stop.set(Some(ImageDecodeError::Cancelled));
            self.owner.cancel(&self.token)
        }

        pub async fn wait(&mut self) -> Result<bool, ImageDecodeError> {
            poll_fn(|context| match self.status.get() {
                BrowserDecodeStatus::Terminal(result) => Poll::Ready(result),
                _ => {
                    *self.waiter.borrow_mut() = Some(context.waker().clone());
                    Poll::Pending
                }
            })
            .await
        }
    }
    impl Drop for BrowserImageDecode {
        fn drop(&mut self) {
            if !matches!(self.status.get(), BrowserDecodeStatus::Terminal(_))
                && self.stop.get().is_none()
            {
                // No fallible/reentrant scene operation in Drop; the retained terminal runner
                // observes this fence before installation and settles the original token only.
                self.stop.set(Some(ImageDecodeError::OwnerDisposed));
            }
        }
    }

    fn prepare(input: &VerifiedImage) -> Result<Prepared, ImageDecodeError> {
        let window = web_sys::window().ok_or(ImageDecodeError::BrowserUnavailable)?;
        let blob = {
            let cache = input
                .cache
                .try_borrow()
                .map_err(|_| ImageDecodeError::OwnerBusy)?;
            let bytes = cache
                .lease_bytes(&input.lease)
                .map_err(ImageDecodeError::Lease)?;
            let copy = Uint8Array::from(bytes);
            let parts = Array::new();
            parts.push(&copy);
            let properties = BlobPropertyBag::new();
            properties.set_type(input.mime());
            Blob::new_with_u8_array_sequence_and_options(&parts, &properties)
                .map_err(|_| ImageDecodeError::BrowserDecode)?
        };
        let options = ImageBitmapOptions::new();
        options.set_color_space_conversion(ColorSpaceConversion::None);
        options.set_premultiply_alpha(PremultiplyAlpha::None);
        let promise = window
            .create_image_bitmap_with_blob_and_image_bitmap_options(&blob, &options)
            .map_err(|_| ImageDecodeError::BrowserUnavailable)?;
        Ok(Prepared { blob, promise })
    }

    async fn decode(
        input: VerifiedImage,
        prepared: Prepared,
        stop: &Cell<Option<ImageDecodeError>>,
    ) -> Result<DecodedImage<CacheKey>, ImageDecodeError> {
        let plan = input.plan();
        let Prepared { blob, promise } = prepared;
        let bitmap_value = JsFuture::from(promise)
            .await
            .map_err(|_| ImageDecodeError::BrowserDecode)?;
        let bitmap = Bitmap(
            bitmap_value
                .dyn_into::<ImageBitmap>()
                .map_err(|_| ImageDecodeError::BrowserDecode)?,
        );
        drop(blob);
        if let Some(reason) = stop.get() {
            return Err(reason);
        }
        input.validate_current()?;
        if (bitmap.0.width(), bitmap.0.height()) != (plan.width, plan.height) {
            return Err(ImageDecodeError::WrongDimensions);
        }
        let document = web_sys::window()
            .and_then(|window| window.document())
            .ok_or(ImageDecodeError::BrowserUnavailable)?;
        let canvas = Scratch(
            document
                .create_element("canvas")
                .map_err(|_| ImageDecodeError::BrowserDecode)?
                .dyn_into::<HtmlCanvasElement>()
                .map_err(|_| ImageDecodeError::BrowserDecode)?,
        );
        canvas.0.set_width(plan.width);
        canvas.0.set_height(plan.height);
        let context = canvas
            .0
            .get_context("2d")
            .map_err(|_| ImageDecodeError::BrowserDecode)?
            .ok_or(ImageDecodeError::BrowserUnavailable)?
            .dyn_into::<CanvasRenderingContext2d>()
            .map_err(|_| ImageDecodeError::BrowserDecode)?;
        context
            .draw_image_with_image_bitmap(&bitmap.0, 0.0, 0.0)
            .map_err(|_| ImageDecodeError::BrowserDecode)?;
        let image_data = context
            .get_image_data(0.0, 0.0, f64::from(plan.width), f64::from(plan.height))
            .map_err(|_| ImageDecodeError::BrowserDecode)?;
        let pixels = image_data.data().0.into_boxed_slice();
        drop(image_data);
        drop(context);
        drop(canvas);
        drop(bitmap);
        input.finish_rgba(plan.width, plan.height, pixels)
    }

    fn settle(
        owner: &DecodeOwner,
        token: &DecodeToken<CacheKey>,
        decoded: Result<DecodedImage<CacheKey>, ImageDecodeError>,
        stop: Option<ImageDecodeError>,
    ) -> Result<bool, ImageDecodeError> {
        if let Some(reason) = stop {
            drop(decoded);
            owner.finish_failed(token)?;
            return Err(reason);
        }
        let image = match decoded {
            Ok(image) => image,
            Err(error) => {
                owner.finish_failed(token)?;
                return Err(error);
            }
        };
        owner.complete_and_present(token, image)
    }
}

#[cfg(target_arch = "wasm32")]
pub use browser::{BrowserDecodeStatus, BrowserImageDecode};
