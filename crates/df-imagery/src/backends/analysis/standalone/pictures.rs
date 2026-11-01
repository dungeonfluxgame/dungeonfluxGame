//! Fixed legacy spatial preparation recipes, bounded by the frozen operator snapshot.
use super::candidate_label;
use super::geometry::SegmentedPerson;
use crate::{config::AnalysisPreparationLimits, runtime::RuntimeError};
use image::{RgbImage, RgbaImage};
use std::io::{Cursor, Write};
/// Covers simultaneous full-source RGB, crop, f32 resize and destination pixel buffers.
const MAXIMUM_WORKING_BYTES_PER_PIXEL: u64 = 24;

struct PictureBuffer {
    bytes: Vec<u8>,
    maximum: usize,
}
impl Write for PictureBuffer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let needed = self
            .bytes
            .len()
            .checked_add(bytes.len())
            .filter(|needed| *needed <= self.maximum)
            .ok_or_else(|| std::io::Error::other("bounded analysis picture"))?;
        if needed > self.bytes.capacity() {
            let target = self
                .bytes
                .capacity()
                .saturating_mul(2)
                .max(needed)
                .min(self.maximum);
            self.bytes
                .try_reserve_exact(target - self.bytes.len())
                .map_err(|_| std::io::Error::other("analysis picture allocation"))?;
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn decode(
    bytes: &[u8],
    limits: &AnalysisPreparationLimits,
    memory: u64,
    retained: u64,
) -> Result<RgbaImage, RuntimeError> {
    let format = image::guess_format(bytes).map_err(|_| RuntimeError::InvalidRecord)?;
    if !matches!(
        format,
        image::ImageFormat::Jpeg | image::ImageFormat::Png | image::ImageFormat::WebP
    ) {
        return Err(RuntimeError::UnsupportedCapability);
    }
    let (width, height) = image::ImageReader::with_format(Cursor::new(bytes), format)
        .into_dimensions()
        .map_err(|_| RuntimeError::InvalidRecord)?;
    let pixels = u64::from(width) * u64::from(height);
    if pixels == 0
        || pixels > limits.max_pixels
        || pixels
            .checked_mul(MAXIMUM_WORKING_BYTES_PER_PIXEL)
            .and_then(|n| n.checked_add(retained))
            .is_none_or(|n| n > memory)
    {
        return Err(RuntimeError::InvalidRecord);
    }
    // The allowance reserves the simultaneously retained RGBA conversion and source bytes.
    let mut bound = image::Limits::no_limits();
    bound.max_image_width = Some(width);
    bound.max_image_height = Some(height);
    bound.max_alloc = Some(memory - retained - pixels * 4);
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(bound);
    reader
        .decode()
        .map(|image| image.into_rgba8())
        .map_err(|_| RuntimeError::InvalidRecord)
}
fn rgb(rgba: RgbaImage, matte: bool) -> RgbImage {
    let mut rgb = RgbImage::new(rgba.width(), rgba.height());
    for (source, target) in rgba.pixels().zip(rgb.pixels_mut()) {
        for (channel, target) in target.0.iter_mut().enumerate() {
            *target = if matte {
                let alpha = u16::from(source[3]);
                ((u16::from(source[channel]) * alpha + 128 * (255 - alpha)) / 255) as u8
            } else {
                source[channel]
            };
        }
    }
    rgb
}
fn encode(
    rgb: RgbImage,
    quality: u8,
    limits: &AnalysisPreparationLimits,
    memory: u64,
    retained: u64,
) -> Result<Vec<u8>, RuntimeError> {
    let available = memory
        .checked_sub(retained)
        .and_then(|n| n.checked_sub(rgb.len() as u64))
        .ok_or(RuntimeError::InvalidRecord)?;
    let maximum = usize::try_from(limits.max_picture_bytes.min(available / 2))
        .map_err(|_| RuntimeError::UnsupportedCapability)?;
    let mut output = PictureBuffer {
        bytes: Vec::new(),
        maximum,
    };
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut output, quality)
        .encode_image(&rgb)
        .map_err(|_| RuntimeError::InvalidRecord)?;
    Ok(output.bytes)
}
pub(super) fn vision(
    bytes: &[u8],
    maximum_width: u32,
    limits: &AnalysisPreparationLimits,
    memory: u64,
    retained: u64,
) -> Result<Vec<u8>, RuntimeError> {
    let picture = rgb(decode(bytes, limits, memory, retained)?, true);
    let width = picture.width().min(maximum_width);
    let height =
        (u64::from(picture.height()) * u64::from(width) / u64::from(picture.width())).max(1) as u32;
    let shown = if (width, height) == picture.dimensions() {
        picture
    } else {
        let shown = image::imageops::resize(
            &picture,
            width,
            height,
            image::imageops::FilterType::Triangle,
        );
        drop(picture);
        shown
    };
    encode(shown, 80, limits, memory, retained)
}
pub(super) fn plate(
    bytes: &[u8],
    limits: &AnalysisPreparationLimits,
    memory: u64,
    retained: u64,
) -> Result<(Vec<u8>, u32, u32), RuntimeError> {
    let picture = rgb(decode(bytes, limits, memory, retained)?, false);
    let long = picture.width().max(picture.height());
    let shown = if long > 2560 {
        let scale = 2560.0 / f64::from(long);
        let scaled = |side: u32| ((f64::from(side) * scale).round() as u32).max(1);
        let shown = image::imageops::resize(
            &picture,
            scaled(picture.width()),
            scaled(picture.height()),
            image::imageops::FilterType::Triangle,
        );
        drop(picture);
        shown
    } else {
        picture
    };
    let (width, height) = shown.dimensions();
    Ok((encode(shown, 90, limits, memory, retained)?, width, height))
}
pub(super) fn crops(
    bytes: &[u8],
    people: &[&SegmentedPerson],
    limits: &AnalysisPreparationLimits,
    memory: u64,
    retained: u64,
) -> Result<Vec<(String, Vec<u8>)>, RuntimeError> {
    let picture = rgb(decode(bytes, limits, memory, retained)?, false);
    let mut held = retained
        .checked_add(picture.len() as u64)
        .ok_or(RuntimeError::InvalidRecord)?;
    let (width, height) = (picture.width() as f32, picture.height() as f32);
    let mut output = Vec::with_capacity(people.len());
    for person in people {
        let body = person.body;
        let margin = (body.right - body.left) * 0.08;
        let bottom = match person.head {
            Some(head) => (head.bottom + (head.bottom - head.top) * 1.5).min(body.bottom),
            None => body.top + (body.bottom - body.top) * 0.4,
        };
        let (left, top, right, bottom) = (
            (body.left - margin).max(0.0),
            body.top,
            (body.right + margin).min(1.0),
            bottom.max(body.top),
        );
        let x = ((left * width).floor() as u32).min(picture.width() - 1);
        let y = ((top * height).floor() as u32).min(picture.height() - 1);
        let crop_width = (((right - left) * width).ceil() as u32).clamp(1, picture.width() - x);
        let crop_height = (((bottom - top) * height).ceil() as u32).clamp(1, picture.height() - y);
        let cropped = image::imageops::crop_imm(&picture, x, y, crop_width, crop_height).to_image();
        let long = cropped.width().max(cropped.height());
        let shown = if long > 448 {
            let scale = 448.0 / f64::from(long);
            let scaled = |side: u32| ((f64::from(side) * scale).round() as u32).max(1);
            let shown = image::imageops::resize(
                &cropped,
                scaled(cropped.width()),
                scaled(cropped.height()),
                image::imageops::FilterType::Triangle,
            );
            drop(cropped);
            shown
        } else {
            cropped
        };
        let jpeg = encode(shown, 85, limits, memory, held)?;
        held = held
            .checked_add(jpeg.capacity() as u64)
            .ok_or(RuntimeError::InvalidRecord)?;
        output.push((candidate_label(person.candidate), jpeg));
    }
    Ok(output)
}

pub(super) fn portrait(
    bytes: &[u8],
    limits: &AnalysisPreparationLimits,
    memory: u64,
    retained: u64,
) -> Result<Vec<u8>, RuntimeError> {
    if image::guess_format(bytes).map_err(|_| RuntimeError::InvalidRecord)?
        != image::ImageFormat::Jpeg
    {
        return vision(bytes, 1024, limits, memory, retained);
    }
    if bytes.len() as u64 > limits.max_picture_bytes {
        return Err(RuntimeError::InvalidRecord);
    }
    let (width, height) =
        image::ImageReader::with_format(Cursor::new(bytes), image::ImageFormat::Jpeg)
            .into_dimensions()
            .map_err(|_| RuntimeError::InvalidRecord)?;
    let pixels = u64::from(width) * u64::from(height);
    if pixels == 0
        || pixels > limits.max_pixels
        || retained
            .checked_add(bytes.len() as u64)
            .is_none_or(|n| n > memory)
    {
        return Err(RuntimeError::InvalidRecord);
    }
    let mut owned = Vec::new();
    owned
        .try_reserve_exact(bytes.len())
        .map_err(|_| RuntimeError::InvalidRecord)?;
    owned.extend_from_slice(bytes);
    Ok(owned)
}

pub(super) fn figure(
    bytes: &[u8],
    limits: &AnalysisPreparationLimits,
    memory: u64,
    retained: u64,
) -> Result<(Vec<u8>, u32, u32), RuntimeError> {
    let jpeg = vision(bytes, 1536, limits, memory, retained)?;
    let (width, height) =
        image::ImageReader::with_format(Cursor::new(&jpeg), image::ImageFormat::Jpeg)
            .into_dimensions()
            .map_err(|_| RuntimeError::InvalidRecord)?;
    Ok((jpeg, width, height))
}
