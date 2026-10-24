//! The party sheet: the heroes' art composed into one reference image (PLAY-V09).
//!
//! The image edit takes at most four reference images, and a plate needs the heroes, the speaker
//! and the place. So the heroes' standing figures (transparent art) are flattened onto plain grey
//! and placed in reading order in one image, wrapping wide parties into rows of three; the prompt names
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
/// Each figure fits within this height; wide sheets wrap rather than dropping figures.
const FIGURE_HEIGHT: u32 = 1024;
const MAXIMUM_FIGURES: usize = 6;
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

/// Composes figures in reading order; a wide party wraps into rows of at most three.
pub(in crate::gameplay) fn party_sheet(
    figures: &[ReferenceImage],
) -> Result<ReferenceImage, SheetError> {
    if figures.is_empty() || figures.len() > MAXIMUM_FIGURES {
        return Err(SheetError::FigureCount);
    }
    let mut pictures = Vec::with_capacity(figures.len());
    let mut row_width = 0_u64;
    for figure in figures {
        let decoded =
            image::load_from_memory_with_format(&figure.bytes, container_of(figure.format))
                .map_err(|_| SheetError::Decode)?;
        let flat = flatten(&decoded);
        if flat.width() == 0 || flat.height() == 0 {
            return Err(SheetError::Decode);
        }
        row_width +=
            (u64::from(flat.width()) * u64::from(FIGURE_HEIGHT) / u64::from(flat.height())).max(1);
        pictures.push(flat);
    }
    row_width += u64::from(GAP) * (figures.len() as u64 - 1);
    let columns = if row_width > u64::from(MAXIMUM_SHEET_WIDTH) {
        figures.len().min(3)
    } else {
        figures.len()
    };
    let rows = figures.len().div_ceil(columns);
    let maximum_width = (MAXIMUM_SHEET_WIDTH - GAP * (columns as u32 - 1)) / columns as u32;
    let scaled: Vec<RgbImage> = pictures
        .iter()
        .map(|picture| {
            let width = (u64::from(picture.width()) * u64::from(FIGURE_HEIGHT)
                / u64::from(picture.height()))
            .clamp(1, u64::from(maximum_width)) as u32;
            let height = (u64::from(picture.height()) * u64::from(width)
                / u64::from(picture.width()))
            .clamp(1, u64::from(FIGURE_HEIGHT)) as u32;
            imageops::resize(picture, width, height, imageops::FilterType::Triangle)
        })
        .collect();
    let width = scaled
        .chunks(columns)
        .map(|row| row.iter().map(RgbImage::width).sum::<u32>() + GAP * (row.len() as u32 - 1))
        .max()
        .ok_or(SheetError::FigureCount)?;
    let height = rows as u32 * FIGURE_HEIGHT + (rows as u32 - 1) * GAP;
    let mut sheet = RgbImage::from_pixel(width, height, Rgb([GREY; 3]));
    for (row, figures) in scaled.chunks(columns).enumerate() {
        let mut left = 0;
        for figure in figures {
            let top = row as u32 * (FIGURE_HEIGHT + GAP) + (FIGURE_HEIGHT - figure.height()) / 2;
            imageops::replace(&mut sheet, figure, i64::from(left), i64::from(top));
            left += figure.width() + GAP;
        }
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

/// `bytes` re-encoded in its own format at a smaller size so it fits `maximum_bytes`, keeping
/// any transparency; `None` when it already fits. A canonical cut-out of a tall figure can come
/// back from the provider just over the edit adapter's reference limit (P6-184).
pub(in crate::gameplay) fn fit_reference(
    bytes: &[u8],
    format: ImageFormat,
    maximum_bytes: usize,
) -> Result<Option<Vec<u8>>, SheetError> {
    if bytes.len() <= maximum_bytes {
        return Ok(None);
    }
    let decoded = image::load_from_memory_with_format(bytes, container_of(format))
        .map_err(|_| SheetError::Decode)?;
    let mut size = bytes.len();
    for _ in 0..4 {
        // Bytes scale roughly with area: shrink each side by the square root of the excess.
        let ratio = (maximum_bytes as f64 / size as f64).sqrt() * 0.9;
        let width = ((f64::from(decoded.width()) * ratio) as u32).max(1);
        let height = ((f64::from(decoded.height()) * ratio) as u32).max(1);
        let scaled = decoded.resize_exact(width, height, imageops::FilterType::Triangle);
        let mut encoded = Vec::new();
        scaled
            .write_to(&mut Cursor::new(&mut encoded), container_of(format))
            .map_err(|_| SheetError::Encode)?;
        if encoded.len() <= maximum_bytes {
            return Ok(Some(encoded));
        }
        size = encoded.len().max(size);
    }
    Err(SheetError::Encode)
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
    fn six_figures_wrap_to_two_rows_of_three_without_exceeding_the_sheet_bound() {
        let colors = [
            [200, 0, 0],
            [0, 200, 0],
            [0, 0, 200],
            [200, 200, 0],
            [0, 200, 200],
            [200, 0, 200],
        ];
        let figures = colors.map(|color| figure(32, 32, 255, color));
        let sheet = party_sheet(&figures).unwrap();
        let decoded = image::load_from_memory(&sheet.bytes).unwrap().to_rgb8();
        assert_eq!(decoded.width(), 3 * FIGURE_HEIGHT + 2 * GAP);
        assert_eq!(decoded.height(), 2 * FIGURE_HEIGHT + GAP);
        assert!(decoded.width() <= MAXIMUM_SHEET_WIDTH);
        for (index, expected) in colors.iter().enumerate() {
            let x = index as u32 % 3 * (FIGURE_HEIGHT + GAP) + FIGURE_HEIGHT / 2;
            let y = index as u32 / 3 * (FIGURE_HEIGHT + GAP) + FIGURE_HEIGHT / 2;
            let pixel = decoded.get_pixel(x, y).0;
            assert!(
                pixel
                    .iter()
                    .zip(expected)
                    .all(|(actual, expected)| actual.abs_diff(*expected) < 12),
                "figure {index}: {pixel:?}"
            );
        }
        let wide = vec![figure(100, 10, 255, [40, 80, 160]); 6];
        let wide = party_sheet(&wide).unwrap();
        let decoded = image::load_from_memory(&wide.bytes).unwrap();
        assert!(decoded.width() <= MAXIMUM_SHEET_WIDTH);
        assert_eq!(decoded.height(), 2 * FIGURE_HEIGHT + GAP);
    }

    #[test]
    fn six_narrow_figures_keep_one_row_when_it_fits() {
        let figures = vec![figure(10, 20, 255, [40, 80, 160]); 6];
        let sheet = party_sheet(&figures).unwrap();
        let decoded = image::load_from_memory(&sheet.bytes).unwrap();
        assert_eq!(decoded.width(), 6 * 512 + 5 * GAP);
        assert_eq!(decoded.height(), FIGURE_HEIGHT);
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
            party_sheet(&vec![one; 7]).unwrap_err(),
            SheetError::FigureCount
        );
        let broken = ReferenceImage::new(ImageFormat::Png, Bytes::from_static(b"not an image"));
        assert_eq!(party_sheet(&[broken]).unwrap_err(), SheetError::Decode);
    }

    #[test]
    fn an_oversized_transparent_reference_is_sent_smaller_with_its_alpha() {
        // Noise compresses badly, so a modest picture is already over a small limit.
        let mut noisy = image::RgbaImage::new(400, 600);
        for (index, pixel) in noisy.pixels_mut().enumerate() {
            let value = (index.wrapping_mul(2_654_435_761) >> 7) as u8;
            *pixel = image::Rgba([value, value.wrapping_mul(3), value.wrapping_mul(7), value]);
        }
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(noisy)
            .write_to(&mut Cursor::new(&mut png), Container::Png)
            .unwrap();
        let limit = png.len() / 2;
        assert_eq!(
            fit_reference(&png, ImageFormat::Png, png.len()).unwrap(),
            None
        );
        let fitted = fit_reference(&png, ImageFormat::Png, limit)
            .unwrap()
            .expect("an oversized reference is re-encoded");
        assert!(fitted.len() <= limit);
        let decoded = image::load_from_memory_with_format(&fitted, Container::Png).unwrap();
        assert!(decoded.color().has_alpha());
        assert!(decoded.width() < 400 && decoded.height() < 600);
    }
}
