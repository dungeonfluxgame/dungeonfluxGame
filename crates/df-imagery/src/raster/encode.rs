//! Final alpha handling and single-pass bounded encoding.
//!
//! The encoded bytes always describe the delivered variant exactly: opaque
//! outputs carry no alpha, preserved alpha round-trips, and matte outputs
//! are flattened onto their explicit color in linear light. Lossless codecs
//! write through the bounded writer; lossy WebP goes through the pinned
//! native encoder with the same output limit on its writer, so encoding
//! stops at the limit instead of measuring afterwards.

use std::io::{Seek, SeekFrom, Write};

use image::{ExtendedColorType, ImageEncoder, RgbImage, RgbaImage};

use super::RasterError;
use super::bounds::{self, LiveBudget};
use super::color::{linear_to_srgb_byte, srgb_byte_to_linear};
use crate::output::{AlphaHandling, Encoding, RasterOutputSpec};

/// Encoded variant with the facts the service publishes.
pub(crate) struct EncodedOutput {
    pub(crate) bytes: Vec<u8>,
    pub(crate) mime: &'static str,
    pub(crate) has_alpha: bool,
}

/// Writer that stops encoding at the output limit instead of growing a
/// post-hoc `Vec`. Both `Write` and `Seek` are implemented because the PNG
/// path requires a seekable stream.
struct BoundedWriter {
    buf: Vec<u8>,
    pos: usize,
    limit: u64,
}

impl BoundedWriter {
    fn new(limit: u64) -> Self {
        Self {
            buf: Vec::new(),
            pos: 0,
            limit,
        }
    }

    fn into_inner(self) -> Vec<u8> {
        self.buf
    }

    fn check_capacity(&self, additional: usize) -> std::io::Result<()> {
        let end = (self.pos as u64).saturating_add(additional as u64);
        if end > self.limit {
            return Err(output_limit_error());
        }
        Ok(())
    }

    fn reserve_exact_for(&mut self, additional: usize) -> std::io::Result<()> {
        // Vec growth can otherwise retain capacity above the logical limit,
        // so every growth requests exactly what the write needs. Failure
        // stays QuotaExceeded so the existing mapper reports OutputTooLarge.
        if self.buf.try_reserve_exact(additional).is_err() {
            return Err(output_limit_error());
        }
        Ok(())
    }
}

impl Write for BoundedWriter {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.check_capacity(data.len())?;
        if self.pos == self.buf.len() {
            self.reserve_exact_for(data.len())?;
            self.buf.extend_from_slice(data);
        } else {
            let end = self.pos.saturating_add(data.len());
            if end > self.buf.len() {
                let grown = end.saturating_sub(self.buf.len());
                self.reserve_exact_for(grown)?;
                self.buf.resize(end, 0);
            }
            self.buf[self.pos..end].copy_from_slice(data);
        }
        self.pos = self.pos.saturating_add(data.len());
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Seek for BoundedWriter {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        // Positions stay within the operator output limit, far below any
        // integer boundary; `i128` keeps the delta math overflow-free.
        let next = match pos {
            SeekFrom::Start(offset) => offset,
            SeekFrom::Current(delta) => {
                (self.pos as i128).saturating_add(delta as i128).max(0) as u64
            }
            SeekFrom::End(delta) => (self.buf.len() as i128)
                .saturating_add(delta as i128)
                .max(0) as u64,
        };
        if next > self.limit {
            return Err(output_limit_error());
        }
        self.pos = next as usize;
        Ok(next)
    }
}

/// The bounded writer's only failure: the output quota.
fn output_limit_error() -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::QuotaExceeded, "output limit")
}

fn map_image_error(error: image::ImageError, budget: &LiveBudget) -> RasterError {
    // The bounded writer's only failure mode is the output quota, so any
    // I/O error out of an encoder writing through it is the limit, whatever
    // kind the pinned encoder preserved. Other encoder failures stay
    // EncodeFailed and the post-write length check keeps its own mapping.
    if matches!(error, image::ImageError::IoError(_)) {
        return RasterError::OutputTooLarge {
            len_bytes: budget.max_output_bytes().saturating_add(1),
        };
    }
    RasterError::EncodeFailed
}

/// Validates the output against the actual raster alpha, flattens when the
/// spec names a matte, and encodes once. Large pixel loops yield to the
/// cancellation check; both the bounded writer and the native WebP writer
/// stop at the output limit, and outputs are checked before and after
/// because encoders cannot yield mid-call.
pub(crate) fn encode_output(
    image: &RgbaImage,
    output: &RasterOutputSpec,
    budget: &LiveBudget,
    check_cancel: super::CancelCheck<'_>,
) -> Result<EncodedOutput, RasterError> {
    let source_has_alpha = image.pixels().any(|pixel| pixel[3] != 255);
    output
        .validate_for_source_alpha(source_has_alpha)
        .map_err(RasterError::InvalidOutput)?;
    let encoding = output.encoding();
    check_cancel()?;
    let (width_px, height_px) = image.dimensions();
    let caller_rgba_bytes = bounds::rgba_bytes(width_px, height_px);
    let (bytes, has_alpha) = match output.alpha() {
        AlphaHandling::PreserveAlpha => match encoding {
            Encoding::Png => (encode_png_rgba(image, budget, check_cancel)?, true),
            Encoding::WebP { quality: None } => {
                (encode_webp_lossless(image, budget, check_cancel)?, true)
            }
            Encoding::WebP {
                quality: Some(quality),
            } => (
                super::webp_lossy::encode_lossy_rgba(
                    image,
                    quality,
                    budget,
                    check_cancel,
                    caller_rgba_bytes,
                )?,
                source_has_alpha,
            ),
            Encoding::Jpeg { .. } => {
                // Unreachable: the constructor refuses JPEG with preserved
                // alpha, but the executor never trusts the caller alone.
                return Err(RasterError::InvalidOutput(
                    crate::output::OutputError::AlphaWithoutMatte,
                ));
            }
        },
        AlphaHandling::Opaque => (
            encode_opaque(image, encoding, budget, check_cancel, caller_rgba_bytes)?,
            false,
        ),
        AlphaHandling::Matte(fill) => {
            let flattened = flatten_onto_matte(image, fill, budget, check_cancel)?;
            let matte_retained_bytes = caller_rgba_bytes.saturating_add(caller_rgba_bytes);
            (
                encode_opaque(
                    &flattened,
                    encoding,
                    budget,
                    check_cancel,
                    matte_retained_bytes,
                )?,
                false,
            )
        }
    };
    check_cancel()?;
    budget.check_output_len(bytes.len() as u64)?;
    Ok(EncodedOutput {
        mime: encoding.format().mime(),
        has_alpha,
        bytes,
    })
}

/// Encodes a raster the caller guarantees is opaque; alpha is dropped by
/// construction, never by the encoder. The RGB copy is reserved before it
/// is allocated and every encoder writes through the bounded writer.
fn encode_opaque(
    image: &RgbaImage,
    encoding: Encoding,
    budget: &LiveBudget,
    check_cancel: super::CancelCheck<'_>,
    caller_retained_bytes: u64,
) -> Result<Vec<u8>, RasterError> {
    let (width_px, height_px) = image.dimensions();
    let pixels = u64::from(width_px).saturating_mul(u64::from(height_px));
    let rgb_bytes = pixels.saturating_mul(3);
    budget.check_retained(caller_retained_bytes, rgb_bytes)?;
    check_cancel()?;
    let mut rgb = RgbImage::new(width_px, height_px);
    for y in 0..height_px {
        if y.is_multiple_of(bounds::CANCEL_POLL_ROWS) {
            check_cancel()?;
        }
        for x in 0..width_px {
            let pixel = image.get_pixel(x, y);
            rgb.put_pixel(x, y, image::Rgb([pixel[0], pixel[1], pixel[2]]));
        }
    }
    check_cancel()?;
    match encoding {
        Encoding::Png => {
            let mut out = BoundedWriter::new(budget.max_output_bytes());
            image::codecs::png::PngEncoder::new(&mut out)
                .write_image(rgb.as_raw(), width_px, height_px, ExtendedColorType::Rgb8)
                .map_err(|error| map_image_error(error, budget))?;
            Ok(out.into_inner())
        }
        Encoding::Jpeg { quality } => {
            let mut out = BoundedWriter::new(budget.max_output_bytes());
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality.value())
                .encode_image(&rgb)
                .map_err(|error| map_image_error(error, budget))?;
            let bytes = out.into_inner();
            budget.check_output_len(bytes.len() as u64)?;
            Ok(bytes)
        }
        Encoding::WebP { quality: None } => {
            let mut out = BoundedWriter::new(budget.max_output_bytes());
            image::codecs::webp::WebPEncoder::new_lossless(&mut out)
                .encode(rgb.as_raw(), width_px, height_px, ExtendedColorType::Rgb8)
                .map_err(|error| map_image_error(error, budget))?;
            let bytes = out.into_inner();
            budget.check_output_len(bytes.len() as u64)?;
            Ok(bytes)
        }
        Encoding::WebP {
            quality: Some(quality),
        } => {
            let total_retained_bytes = caller_retained_bytes.saturating_add(rgb_bytes);
            let bytes = super::webp_lossy::encode_lossy_rgb(
                &rgb,
                quality,
                budget,
                check_cancel,
                total_retained_bytes,
            )?;
            budget.check_output_len(bytes.len() as u64)?;
            Ok(bytes)
        }
    }
}

fn encode_png_rgba(
    image: &RgbaImage,
    budget: &LiveBudget,
    check_cancel: super::CancelCheck<'_>,
) -> Result<Vec<u8>, RasterError> {
    check_cancel()?;
    let (width_px, height_px) = image.dimensions();
    let mut out = BoundedWriter::new(budget.max_output_bytes());
    image::codecs::png::PngEncoder::new(&mut out)
        .write_image(
            image.as_raw(),
            width_px,
            height_px,
            ExtendedColorType::Rgba8,
        )
        .map_err(|error| map_image_error(error, budget))?;
    Ok(out.into_inner())
}

fn encode_webp_lossless(
    image: &RgbaImage,
    budget: &LiveBudget,
    check_cancel: super::CancelCheck<'_>,
) -> Result<Vec<u8>, RasterError> {
    check_cancel()?;
    let (width_px, height_px) = image.dimensions();
    let mut out = BoundedWriter::new(budget.max_output_bytes());
    image::codecs::webp::WebPEncoder::new_lossless(&mut out)
        .encode(
            image.as_raw(),
            width_px,
            height_px,
            ExtendedColorType::Rgba8,
        )
        .map_err(|error| map_image_error(error, budget))?;
    let bytes = out.into_inner();
    budget.check_output_len(bytes.len() as u64)?;
    Ok(bytes)
}

/// Flattens straight-alpha pixels onto the explicit opaque matte color in
/// linear light with premultiplied alpha, so translucent edges keep their
/// hue instead of fringing toward the matte.
fn flatten_onto_matte(
    image: &RgbaImage,
    matte: crate::output::Rgb,
    budget: &LiveBudget,
    check_cancel: super::CancelCheck<'_>,
) -> Result<RgbaImage, RasterError> {
    let (width_px, height_px) = image.dimensions();
    budget.check_retained(
        bounds::rgba_bytes(width_px, height_px),
        bounds::rgba_bytes(width_px, height_px),
    )?;
    check_cancel()?;
    let matte_linear = [
        srgb_byte_to_linear(matte.red),
        srgb_byte_to_linear(matte.green),
        srgb_byte_to_linear(matte.blue),
    ];
    let mut flattened = RgbaImage::new(width_px, height_px);
    for y in 0..height_px {
        if y.is_multiple_of(bounds::CANCEL_POLL_ROWS) {
            check_cancel()?;
        }
        for x in 0..width_px {
            let pixel = image.get_pixel(x, y);
            let alpha = f32::from(pixel[3]) / 255.0;
            let mut mixed = [0_u8; 4];
            for channel in 0..3 {
                let foreground = srgb_byte_to_linear(pixel[channel]);
                let value = foreground * alpha + matte_linear[channel] * (1.0 - alpha);
                mixed[channel] = linear_to_srgb_byte(value);
            }
            mixed[3] = 255;
            flattened.put_pixel(x, y, image::Rgba(mixed));
        }
    }
    Ok(flattened)
}

#[cfg(test)]
mod encode_tests;
