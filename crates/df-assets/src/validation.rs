//! Bounded validation and metadata-stripping re-encoding for uploaded raster images.

use image::{DynamicImage, ImageFormat, ImageReader};
use std::io::Cursor;

pub const MAX_UPLOAD_BYTES: usize = 10 * 1024 * 1024;
pub const MAX_IMAGE_PIXELS: u64 = 20_000_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImageRejection {
    TooLarge,
    UnsupportedFormat,
    MimeMismatch,
    ExtensionMismatch,
    Animated,
    InvalidHeader,
    TooManyPixels,
    DecodeFailed,
    EncodeFailed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedImage {
    pub bytes: Vec<u8>,
    pub mime: &'static str,
    pub width_px: u32,
    pub height_px: u32,
}

/// Checks type and dimensions before decoding, then re-encodes to discard source metadata.
pub fn validate_image(
    bytes: &[u8],
    declared_mime: &str,
    extension: &str,
) -> Result<ValidatedImage, ImageRejection> {
    if bytes.len() > MAX_UPLOAD_BYTES {
        return Err(ImageRejection::TooLarge);
    }
    let format = image::guess_format(bytes).map_err(|_| ImageRejection::UnsupportedFormat)?;
    let (mime, expected_extension) = match format {
        ImageFormat::Png => ("image/png", "png"),
        ImageFormat::Jpeg => ("image/jpeg", "jpg"),
        ImageFormat::WebP => ("image/webp", "webp"),
        _ => return Err(ImageRejection::UnsupportedFormat),
    };
    if declared_mime != mime {
        return Err(ImageRejection::MimeMismatch);
    }
    let extension = extension.trim_start_matches('.');
    if !(extension.eq_ignore_ascii_case(expected_extension)
        || (format == ImageFormat::Jpeg && extension.eq_ignore_ascii_case("jpeg")))
    {
        return Err(ImageRejection::ExtensionMismatch);
    }
    if is_animated(bytes, format)? {
        return Err(ImageRejection::Animated);
    }
    let reader = ImageReader::with_format(Cursor::new(bytes), format);
    let (width_px, height_px) = reader
        .into_dimensions()
        .map_err(|_| ImageRejection::InvalidHeader)?;
    let pixels = u64::from(width_px)
        .checked_mul(u64::from(height_px))
        .ok_or(ImageRejection::TooManyPixels)?;
    if pixels == 0 || pixels > MAX_IMAGE_PIXELS {
        return Err(ImageRejection::TooManyPixels);
    }
    let decoded = image::load_from_memory_with_format(bytes, format)
        .map_err(|_| ImageRejection::DecodeFailed)?;
    let output = encode_without_metadata(decoded, format)?;
    Ok(ValidatedImage {
        bytes: output,
        mime,
        width_px,
        height_px,
    })
}

fn encode_without_metadata(
    image: DynamicImage,
    format: ImageFormat,
) -> Result<Vec<u8>, ImageRejection> {
    let mut output = Cursor::new(Vec::new());
    image
        .write_to(&mut output, format)
        .map_err(|_| ImageRejection::EncodeFailed)?;
    Ok(output.into_inner())
}

fn is_animated(bytes: &[u8], format: ImageFormat) -> Result<bool, ImageRejection> {
    match format {
        ImageFormat::Png => png_has_animation_control(bytes),
        ImageFormat::WebP => webp_has_animation_flag(bytes),
        ImageFormat::Jpeg => {
            if bytes.ends_with(&[0xff, 0xd9]) {
                Ok(false)
            } else {
                Err(ImageRejection::InvalidHeader)
            }
        }
        _ => Ok(true),
    }
}

fn png_has_animation_control(bytes: &[u8]) -> Result<bool, ImageRejection> {
    let mut offset = 8_usize;
    while offset < bytes.len() {
        let header_end = offset.checked_add(8).ok_or(ImageRejection::InvalidHeader)?;
        if header_end > bytes.len() {
            return Err(ImageRejection::InvalidHeader);
        }
        let length = u32::from_be_bytes(
            bytes[offset..offset + 4]
                .try_into()
                .map_err(|_| ImageRejection::InvalidHeader)?,
        ) as usize;
        let chunk_end = header_end
            .checked_add(length)
            .and_then(|value| value.checked_add(4))
            .ok_or(ImageRejection::InvalidHeader)?;
        if chunk_end > bytes.len() {
            return Err(ImageRejection::InvalidHeader);
        }
        if &bytes[offset + 4..header_end] == b"acTL" {
            return Ok(true);
        }
        if &bytes[offset + 4..header_end] == b"IEND" {
            return if chunk_end == bytes.len() {
                Ok(false)
            } else {
                Err(ImageRejection::InvalidHeader)
            };
        }
        offset = chunk_end;
    }
    Err(ImageRejection::InvalidHeader)
}

fn webp_has_animation_flag(bytes: &[u8]) -> Result<bool, ImageRejection> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WEBP" {
        return Err(ImageRejection::InvalidHeader);
    }
    let declared_len = u32::from_le_bytes(
        bytes[4..8]
            .try_into()
            .map_err(|_| ImageRejection::InvalidHeader)?,
    ) as usize;
    if declared_len.checked_add(8) != Some(bytes.len()) {
        return Err(ImageRejection::InvalidHeader);
    }
    let mut offset = 12_usize;
    while offset + 8 <= bytes.len() {
        let kind = &bytes[offset..offset + 4];
        let length = u32::from_le_bytes(
            bytes[offset + 4..offset + 8]
                .try_into()
                .map_err(|_| ImageRejection::InvalidHeader)?,
        ) as usize;
        let data_start = offset + 8;
        let data_end = data_start
            .checked_add(length)
            .ok_or(ImageRejection::InvalidHeader)?;
        if data_end > bytes.len() {
            return Err(ImageRejection::InvalidHeader);
        }
        if kind == b"VP8X" {
            let flags = *bytes.get(data_start).ok_or(ImageRejection::InvalidHeader)?;
            return Ok(flags & 0x02 != 0);
        }
        if kind == b"VP8 " || kind == b"VP8L" {
            return Ok(false);
        }
        offset = data_end + (length & 1);
    }
    Err(ImageRejection::InvalidHeader)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_bytes(width: u32, height: u32) -> Vec<u8> {
        let image = DynamicImage::ImageRgba8(image::RgbaImage::new(width, height));
        let mut output = Cursor::new(Vec::new());
        image
            .write_to(&mut output, ImageFormat::Png)
            .expect("encode fixture");
        output.into_inner()
    }

    #[test]
    fn valid_png_is_decoded_and_reencoded() {
        let bytes = png_bytes(2, 3);
        let validated = validate_image(&bytes, "image/png", "png").expect("valid image");
        assert_eq!(validated.mime, "image/png");
        assert_eq!((validated.width_px, validated.height_px), (2, 3));
        assert!(image::load_from_memory(&validated.bytes).is_ok());
    }

    #[test]
    fn png_with_trailing_polyglot_payload_is_refused() {
        let mut bytes = png_bytes(1, 1);
        bytes.extend_from_slice(b"<script>alert(1)</script>");
        assert_eq!(
            validate_image(&bytes, "image/png", "png"),
            Err(ImageRejection::InvalidHeader)
        );
    }

    #[test]
    fn png_declared_as_jpeg_is_refused() {
        assert_eq!(
            validate_image(&png_bytes(1, 1), "image/jpeg", "jpg"),
            Err(ImageRejection::MimeMismatch)
        );
    }

    #[test]
    fn image_over_pixel_limit_is_refused_from_its_header() {
        let mut bytes = png_bytes(1, 1);
        bytes[16..20].copy_from_slice(&5000_u32.to_be_bytes());
        bytes[20..24].copy_from_slice(&5000_u32.to_be_bytes());
        let ihdr_crc = png_crc32(&bytes[12..29]);
        bytes[29..33].copy_from_slice(&ihdr_crc.to_be_bytes());
        assert_eq!(
            validate_image(&bytes, "image/png", "png"),
            Err(ImageRejection::TooManyPixels)
        );
    }

    #[test]
    fn upload_over_byte_limit_is_refused_before_format_parsing() {
        assert_eq!(
            validate_image(&vec![0; MAX_UPLOAD_BYTES + 1], "image/png", "png"),
            Err(ImageRejection::TooLarge)
        );
    }

    #[test]
    fn animated_png_and_webp_are_refused() {
        let mut png = png_bytes(1, 1);
        let actl = png_chunk(b"acTL", &[0, 0, 0, 2, 0, 0, 0, 0]);
        let idat = png
            .windows(4)
            .position(|window| window == b"IDAT")
            .expect("PNG data chunk")
            - 4;
        png.splice(idat..idat, actl);
        assert_eq!(
            validate_image(&png, "image/png", "png"),
            Err(ImageRejection::Animated)
        );

        let mut webp = b"RIFF\x16\0\0\0WEBPVP8X\x0a\0\0\0".to_vec();
        webp.push(0x02);
        webp.extend_from_slice(&[0; 9]);
        assert_eq!(webp.len(), 30);
        assert_eq!(
            validate_image(&webp, "image/webp", "webp"),
            Err(ImageRejection::Animated)
        );
    }

    #[test]
    fn wrong_extension_is_refused() {
        assert_eq!(
            validate_image(&png_bytes(1, 1), "image/png", "jpg"),
            Err(ImageRejection::ExtensionMismatch)
        );
    }

    #[test]
    fn png_exif_metadata_is_removed_before_promotion() {
        let mut bytes = png_bytes(1, 1);
        let metadata = b"Exif\0\0GPS coordinates";
        let chunk = png_chunk(b"eXIf", metadata);
        let idat = bytes
            .windows(4)
            .position(|window| window == b"IDAT")
            .expect("PNG data chunk")
            - 4;
        bytes.splice(idat..idat, chunk);

        let validated = validate_image(&bytes, "image/png", "png").expect("valid image");

        assert!(!validated.bytes.windows(3).any(|window| window == b"GPS"));
    }

    fn png_crc32(bytes: &[u8]) -> u32 {
        let mut crc = !0_u32;
        for byte in bytes {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = (crc >> 1) ^ (0xedb8_8320 & (0_u32.wrapping_sub(crc & 1)));
            }
        }
        !crc
    }

    fn png_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut crc_input = kind.to_vec();
        crc_input.extend_from_slice(data);
        let mut chunk = Vec::new();
        chunk.extend_from_slice(&(data.len() as u32).to_be_bytes());
        chunk.extend_from_slice(kind);
        chunk.extend_from_slice(data);
        chunk.extend_from_slice(&png_crc32(&crc_input).to_be_bytes());
        chunk
    }
}
