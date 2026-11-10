//! A cheap check that a generated picture shows something. An image model asked for a painting
//! sometimes returns only the blank surface it was to be painted on: a flat rectangle with a pale
//! edge. Such a picture has almost no variation of light across its middle and almost no edges
//! there, which this measures on a small copy of the picture; it judges nothing about what a
//! picture shows, only that it is not empty.

use image::GenericImageView;
use image::imageops::FilterType;

/// The longer side of the copy that is measured, in pixels.
const MEASURED_SIDE_PX: u32 = 128;
/// The middle of the picture that is measured, in percent of each side: the edge, where a blank
/// surface shows its margin, is left out.
const CENTRAL_PERCENT: u32 = 60;
/// A picture whose light varies by no more than this (the standard deviation of luminance, 0 to 255
/// scale) across its middle may be blank.
pub const BLANK_MAXIMUM_DEVIATION: f32 = 6.0;
/// A picture whose neighbouring pixels differ by no more than this on average (0 to 255 scale)
/// across its middle may be blank.
pub const BLANK_MAXIMUM_EDGE_ENERGY: f32 = 1.5;

/// What was measured of the middle of a picture.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PictureStats {
    /// The standard deviation of luminance, 0 to 255 scale.
    pub luminance_deviation: f32,
    /// The mean absolute difference between neighbouring pixels, 0 to 255 scale.
    pub edge_energy: f32,
}

impl PictureStats {
    /// Whether the middle of the picture has too little variation and too few edges to show
    /// anything: both are at or below their bounds.
    pub fn is_near_blank(&self) -> bool {
        self.luminance_deviation <= BLANK_MAXIMUM_DEVIATION
            && self.edge_energy <= BLANK_MAXIMUM_EDGE_ENERGY
    }
}

/// The picture is not one that decodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PictureCheckError;

impl std::fmt::Display for PictureCheckError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("the picture does not decode")
    }
}

impl std::error::Error for PictureCheckError {}

/// The variation of light and the edges across the middle of the encoded picture `bytes`.
pub fn measure_picture(bytes: &[u8]) -> Result<PictureStats, PictureCheckError> {
    let picture = image::load_from_memory(bytes).map_err(|_| PictureCheckError)?;
    let small = picture.resize(MEASURED_SIDE_PX, MEASURED_SIDE_PX, FilterType::Triangle);
    let (width, height) = small.dimensions();
    let margin = |side: u32| side * (100 - CENTRAL_PERCENT) / 200;
    let (left, top) = (margin(width), margin(height));
    let (inner_width, inner_height) = (width - 2 * left, height - 2 * top);
    let middle = small
        .crop_imm(left, top, inner_width.max(1), inner_height.max(1))
        .to_luma8();
    let (columns, rows) = middle.dimensions();
    let count = (u64::from(columns) * u64::from(rows)) as f32;
    let sum: f32 = middle.pixels().map(|pixel| f32::from(pixel.0[0])).sum();
    let mean = sum / count;
    let variance: f32 = middle
        .pixels()
        .map(|pixel| (f32::from(pixel.0[0]) - mean).powi(2))
        .sum::<f32>()
        / count;
    let (mut edges, mut pairs) = (0.0_f32, 0.0_f32);
    for y in 0..rows {
        for x in 0..columns {
            let here = f32::from(middle.get_pixel(x, y).0[0]);
            if x + 1 < columns {
                edges += (here - f32::from(middle.get_pixel(x + 1, y).0[0])).abs();
                pairs += 1.0;
            }
            if y + 1 < rows {
                edges += (here - f32::from(middle.get_pixel(x, y + 1).0[0])).abs();
                pairs += 1.0;
            }
        }
    }
    Ok(PictureStats {
        luminance_deviation: variance.sqrt(),
        edge_energy: if pairs > 0.0 { edges / pairs } else { 0.0 },
    })
}
