//! The party sheet: the heroes' art composed into one reference image (PLAY-V09).
//!
//! The image edit takes at most four reference images, and a plate needs the heroes, the speaker
//! and the place. So the heroes' standing figures (transparent art) are flattened onto plain grey
//! and placed side by side in one image, left to right in the order given; the plate prompt names
//! each hero by their position on the sheet. The composition is deterministic, so the same heroes
//! always give the same bytes and the same cache key.

use std::io::Cursor;

use bytes::Bytes;
use df_providers::ImageFormat;
use image::{
    DynamicImage, ImageFormat as Container, Rgb, RgbImage, codecs::jpeg::JpegEncoder, imageops,
};

use super::references::ReferenceImage;

const GREY: u8 = 128;
/// Neutral grey between the photographs: they read as separate photographs, not one group pose.
const GAP: u32 = 24;
const JPEG_QUALITY: u8 = 90;
/// Each figure is scaled to this height; the sheet is as wide as the figures side by side.
const FIGURE_HEIGHT: u32 = 1024;
const MAXIMUM_FIGURES: usize = 3;
const MAXIMUM_SHEET_WIDTH: u32 = 4096;

/// Why a sheet could not be made.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay) enum SheetError {
    /// No figure, or more than a sheet holds.
    FigureCount,
    /// A figure is not an image this server can decode.
    Decode,
    /// The composed sheet could not be encoded or is too large.
    Encode,
}

fn container_of(format: ImageFormat) -> Container {
    match format {
        ImageFormat::Png => Container::Png,
        ImageFormat::Jpeg => Container::Jpeg,
        ImageFormat::Webp => Container::WebP,
    }
}

/// Flattens an image with transparency onto grey.
fn flatten(image: &DynamicImage) -> RgbImage {
    let rgba = image.to_rgba8();
    let mut flat = RgbImage::from_pixel(rgba.width(), rgba.height(), Rgb([GREY; 3]));
    for (target, source) in flat.pixels_mut().zip(rgba.pixels()) {
        let alpha = u16::from(source.0[3]);
        for channel in 0..3 {
            let blended =
                (u16::from(source.0[channel]) * alpha + u16::from(GREY) * (255 - alpha)) / 255;
            target.0[channel] = u8::try_from(blended).unwrap_or(u8::MAX);
        }
    }
    flat
}

/// Composes `figures` left to right onto one grey sheet and encodes it as a JPEG reference.
pub(in crate::gameplay) fn party_sheet(
    figures: &[ReferenceImage],
) -> Result<ReferenceImage, SheetError> {
    if figures.is_empty() || figures.len() > MAXIMUM_FIGURES {
        return Err(SheetError::FigureCount);
    }
    let mut scaled = Vec::with_capacity(figures.len());
    for figure in figures {
        let decoded =
            image::load_from_memory_with_format(&figure.bytes, container_of(figure.format))
                .map_err(|_| SheetError::Decode)?;
        let flat = flatten(&decoded);
        if flat.width() == 0 || flat.height() == 0 {
            return Err(SheetError::Decode);
        }
        let width =
            (u64::from(flat.width()) * u64::from(FIGURE_HEIGHT) / u64::from(flat.height())).max(1);
        let width = u32::try_from(width).map_err(|_| SheetError::Decode)?;
        scaled.push(imageops::resize(
            &flat,
            width,
            FIGURE_HEIGHT,
            imageops::FilterType::Triangle,
        ));
    }
    let total: u32 = scaled.iter().map(RgbImage::width).sum::<u32>()
        + GAP * u32::try_from(scaled.len().saturating_sub(1)).unwrap_or(0);
    if total > MAXIMUM_SHEET_WIDTH {
        return Err(SheetError::Encode);
    }
    let mut sheet = RgbImage::from_pixel(total, FIGURE_HEIGHT, Rgb([GREY; 3]));
    let mut left = 0;
    for figure in &scaled {
        imageops::replace(&mut sheet, figure, i64::from(left), 0);
        left += figure.width() + GAP;
    }
    let mut encoded = Vec::new();
    JpegEncoder::new_with_quality(&mut Cursor::new(&mut encoded), JPEG_QUALITY)
        .encode_image(&sheet)
        .map_err(|_| SheetError::Encode)?;
    Ok(ReferenceImage::new(ImageFormat::Jpeg, Bytes::from(encoded)))
}

/// A picture as the vision model is shown it: at most `maximum_width` pixels wide, a JPEG.
pub(in crate::gameplay) fn vision_picture(
    bytes: &[u8],
    format: ImageFormat,
    maximum_width: u32,
) -> Result<Vec<u8>, SheetError> {
    let decoded = image::load_from_memory_with_format(bytes, container_of(format))
        .map_err(|_| SheetError::Decode)?;
    let picture = flatten(&decoded);
    let width = picture.width().min(maximum_width);
    if width == 0 || picture.height() == 0 {
        return Err(SheetError::Decode);
    }
    let height =
        u32::try_from(u64::from(picture.height()) * u64::from(width) / u64::from(picture.width()))
            .map_err(|_| SheetError::Decode)?
            .max(1);
    let scaled = imageops::resize(&picture, width, height, imageops::FilterType::Triangle);
    let mut encoded = Vec::new();
    JpegEncoder::new_with_quality(&mut Cursor::new(&mut encoded), 80)
        .encode_image(&scaled)
        .map_err(|_| SheetError::Encode)?;
    Ok(encoded)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn figure(width: u32, height: u32, alpha: u8, color: [u8; 3]) -> ReferenceImage {
        let mut image = image::RgbaImage::new(width, height);
        for pixel in image.pixels_mut() {
            *pixel = image::Rgba([color[0], color[1], color[2], alpha]);
        }
        let mut bytes = Vec::new();
        DynamicImage::ImageRgba8(image)
            .write_to(&mut Cursor::new(&mut bytes), Container::Png)
            .unwrap();
        ReferenceImage::new(ImageFormat::Png, Bytes::from(bytes))
    }

    #[test]
    fn figures_stand_side_by_side_on_grey_in_the_order_given() {
        let sheet = party_sheet(&[
            figure(100, 200, 255, [200, 0, 0]),
            figure(100, 200, 0, [0, 200, 0]),
        ])
        .unwrap();
        assert_eq!(sheet.format, ImageFormat::Jpeg);
        let decoded = image::load_from_memory_with_format(&sheet.bytes, Container::Jpeg)
            .unwrap()
            .to_rgb8();
        assert_eq!(
            (decoded.width(), decoded.height()),
            (1024 + GAP, FIGURE_HEIGHT)
        );
        let left = decoded.get_pixel(256, 512).0;
        let gap = decoded.get_pixel(512 + GAP / 2, 512).0;
        assert!(
            gap.iter().all(|channel| channel.abs_diff(GREY) < 12),
            "separate photographs are divided by neutral grey: {gap:?}"
        );
        let right = decoded.get_pixel(512 + GAP + 256, 512).0;
        assert!(
            left[0] > 150 && left[1] < 60,
            "the first figure is red: {left:?}"
        );
        assert!(
            right.iter().all(|channel| channel.abs_diff(GREY) < 12),
            "a fully transparent figure leaves grey: {right:?}"
        );
    }

    #[test]
    fn a_plate_is_shown_to_the_vision_model_as_a_small_jpeg() {
        let big = figure(2560, 1440, 255, [30, 40, 50]);
        let small = vision_picture(&big.bytes, ImageFormat::Png, 1024).unwrap();
        let decoded = image::load_from_memory_with_format(&small, Container::Jpeg).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (1024, 576));
        assert_eq!(
            vision_picture(b"not an image", ImageFormat::Png, 1024),
            Err(SheetError::Decode)
        );
    }

    #[test]
    fn the_same_figures_give_the_same_bytes() {
        let figures = [figure(80, 160, 255, [10, 20, 30])];
        assert_eq!(
            party_sheet(&figures).unwrap().sha256,
            party_sheet(&figures).unwrap().sha256
        );
    }

    #[test]
    fn no_figures_too_many_figures_and_undecodable_figures_are_refused() {
        assert_eq!(party_sheet(&[]).unwrap_err(), SheetError::FigureCount);
        let one = figure(10, 20, 255, [1, 2, 3]);
        assert_eq!(
            party_sheet(&[one.clone(), one.clone(), one.clone(), one.clone()]).unwrap_err(),
            SheetError::FigureCount
        );
        let broken = ReferenceImage::new(ImageFormat::Png, Bytes::from_static(b"not an image"));
        assert_eq!(party_sheet(&[broken]).unwrap_err(), SheetError::Decode);
    }
}
