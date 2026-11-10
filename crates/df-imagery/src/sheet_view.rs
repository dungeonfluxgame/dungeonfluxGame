//! One view of a character reference sheet. A sheet is one picture with several views of the same
//! person side by side (a front close-up, a three-quarter view, a full-length view, left to
//! right); an image model that is asked to edit a picture after such a sheet can read the views as
//! separate people and paint the person two or three times. A single view cannot. It is pure
//! pixels: the sheet comes in as encoded bytes and one PNG comes out; it makes no call and reads no
//! file.
//!
//! A picture that is not wide enough to hold the views side by side (a single portrait) is not a
//! multi-view sheet: it has no views to pick from and is answered as `None`, to be used as it is.

use std::io::Cursor;

use image::{ImageFormat, imageops};

/// How many views a sheet holds, side by side.
pub const SHEET_VIEWS: u32 = 3;

/// A picture is a multi-view sheet when it is at least this much wider than tall, as a ratio of
/// 3 to 2 (`width * 2 >= height * 3`): three portrait views side by side are about 1.8 to 1.
const MINIMUM_WIDTH_OVER_HEIGHT: (u32, u32) = (3, 2);

/// Which of the sheet's views to take, left to right.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SheetView {
    /// The front close-up, the left third: the face as it is seen head on.
    Front,
    /// The three-quarter view, the middle third.
    ThreeQuarter,
    /// The full-length view, the right third: the whole body.
    FullLength,
}

impl SheetView {
    const fn index(self) -> u32 {
        match self {
            Self::Front => 0,
            Self::ThreeQuarter => 1,
            Self::FullLength => 2,
        }
    }
}

/// Why a view could not be made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SheetViewError {
    /// The bytes are not a picture this crate decodes.
    Picture,
    /// The view could not be written as a PNG.
    Encode,
}

impl std::fmt::Display for SheetViewError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Picture => "the sheet is not a picture that can be read",
            Self::Encode => "the view could not be written as a picture",
        })
    }
}

impl std::error::Error for SheetViewError {}

/// The `view` of the multi-view sheet `bytes`, as a PNG; `None` when the picture is not wide enough
/// to be one (it has a single view, which is used whole).
pub fn sheet_view(bytes: &[u8], view: SheetView) -> Result<Option<Vec<u8>>, SheetViewError> {
    let picture = image::load_from_memory(bytes).map_err(|_| SheetViewError::Picture)?;
    let (width, height) = (picture.width(), picture.height());
    let (over, under) = MINIMUM_WIDTH_OVER_HEIGHT;
    if width.saturating_mul(under) < height.saturating_mul(over) || width < SHEET_VIEWS {
        return Ok(None);
    }
    let band = width / SHEET_VIEWS;
    let left = band * view.index();
    // The last view takes what the division left over.
    let taken = if view.index() + 1 == SHEET_VIEWS {
        width - left
    } else {
        band
    };
    let cropped = imageops::crop_imm(&picture.to_rgba8(), left, 0, taken, height).to_image();
    let mut out = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(cropped)
        .write_to(&mut out, ImageFormat::Png)
        .map_err(|_| SheetViewError::Encode)?;
    Ok(Some(out.into_inner()))
}

#[cfg(test)]
mod tests {
    use image::{Rgba, RgbaImage};

    use super::*;

    /// A sheet of `width` by `height` whose thirds are red, green and blue.
    fn sheet(width: u32, height: u32) -> Vec<u8> {
        let image = RgbaImage::from_fn(width, height, |x, _| match x * SHEET_VIEWS / width {
            0 => Rgba([255, 0, 0, 255]),
            1 => Rgba([0, 255, 0, 255]),
            _ => Rgba([0, 0, 255, 255]),
        });
        let mut out = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image)
            .write_to(&mut out, ImageFormat::Png)
            .unwrap();
        out.into_inner()
    }

    fn decoded(bytes: &[u8]) -> RgbaImage {
        image::load_from_memory(bytes).unwrap().to_rgba8()
    }

    #[test]
    fn each_view_of_a_three_view_sheet_is_its_own_third() {
        let bytes = sheet(1665, 928);
        for (view, colour, width) in [
            (SheetView::Front, [255, 0, 0, 255], 555),
            (SheetView::ThreeQuarter, [0, 255, 0, 255], 555),
            (SheetView::FullLength, [0, 0, 255, 255], 555),
        ] {
            let cropped = decoded(
                &sheet_view(&bytes, view)
                    .unwrap()
                    .expect("a multi-view sheet"),
            );
            assert_eq!(cropped.dimensions(), (width, 928), "{view:?}");
            assert!(
                cropped.pixels().all(|pixel| pixel.0 == colour),
                "{view:?} holds only its own view"
            );
        }
    }

    #[test]
    fn a_single_view_picture_is_not_cropped() {
        // A portrait, a square and a picture just under 3 to 2 each have one view.
        for (width, height) in [(832, 1248), (1024, 1024), (1380, 928)] {
            assert_eq!(
                sheet_view(&sheet(width, height), SheetView::Front),
                Ok(None),
                "{width}x{height}"
            );
        }
        assert!(
            sheet_view(&sheet(1392, 928), SheetView::Front)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn bytes_that_are_not_a_picture_are_refused() {
        assert_eq!(
            sheet_view(b"not a picture", SheetView::Front),
            Err(SheetViewError::Picture)
        );
    }
}
