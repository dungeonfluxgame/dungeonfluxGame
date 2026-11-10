//! A gallery sheet: many pictures tiled into one, so a backend that takes only a few reference
//! pictures can be shown a whole collection as a single picture. It is pure pixels: the pictures
//! come in as encoded bytes, in the order they are laid out, and one PNG comes out. It draws no
//! labels and no text, makes no call and reads no file.
//!
//! Each picture is scaled to fit its tile whole (its shape kept) and centred on the background, so
//! a wide painting and a tall one both show in full. The columns derive from the count so that the
//! sheet comes out roughly square, unless the spec fixes them; tiles fill left to right, top to
//! bottom, and a last row that is not full leaves its empty cells as background.

use std::io::Cursor;

use image::imageops::FilterType;
use image::{DynamicImage, ImageFormat, Rgb, RgbImage};
use serde::{Deserialize, Serialize};

use crate::output::MAXIMUM_OUTPUT_PIXELS;

/// The most pictures one sheet holds.
pub const MAXIMUM_GALLERY_PICTURES: usize = 256;
/// The widest or tallest a tile or the gap may be, in pixels.
pub const MAXIMUM_TILE_SIDE_PX: u32 = 4_096;

/// How a gallery sheet is laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GallerySpec {
    /// The columns; derived from the count when none, so the sheet is roughly square.
    pub columns: Option<u32>,
    pub tile_width_px: u32,
    pub tile_height_px: u32,
    /// The space between tiles and round the edge.
    pub gap_px: u32,
    /// The colour of the sheet behind and between the tiles: `[red, green, blue]`.
    pub background: [u8; 3],
}

impl Default for GallerySpec {
    /// Sixteen-by-nine tiles of 448 by 252 with a 12 px gap on mid-grey.
    fn default() -> Self {
        Self {
            columns: None,
            tile_width_px: 448,
            tile_height_px: 252,
            gap_px: 12,
            background: [128, 128, 128],
        }
    }
}

/// Why a gallery sheet could not be made.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "cause", rename_all = "snake_case")]
pub enum GalleryError {
    /// No pictures to lay out.
    Empty,
    /// More pictures than [`MAXIMUM_GALLERY_PICTURES`].
    TooMany { count: usize },
    /// A tile or the gap is zero or past [`MAXIMUM_TILE_SIDE_PX`], or the columns are zero.
    InvalidSpec,
    /// The sheet would pass [`MAXIMUM_OUTPUT_PIXELS`].
    TooLarge,
    /// The picture at `index` (from zero) is not one that decodes.
    Picture { index: usize },
    /// The sheet could not be encoded.
    Encode,
}

impl std::fmt::Display for GalleryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => formatter.write_str("a gallery needs at least one picture"),
            Self::TooMany { count } => write!(
                formatter,
                "{count} pictures are more than a gallery of {MAXIMUM_GALLERY_PICTURES} holds"
            ),
            Self::InvalidSpec => formatter.write_str(
                "the gallery's columns, tile size and gap must be above zero and within bounds",
            ),
            Self::TooLarge => formatter.write_str("the gallery sheet would be too large a picture"),
            Self::Picture { index } => {
                write!(formatter, "picture {index} of the gallery does not decode")
            }
            Self::Encode => formatter.write_str("the gallery sheet could not be encoded"),
        }
    }
}

impl std::error::Error for GalleryError {}

/// The columns and rows `count` pictures are laid out in under `spec`.
pub fn gallery_grid(count: usize, spec: &GallerySpec) -> Result<(u32, u32), GalleryError> {
    if count == 0 {
        return Err(GalleryError::Empty);
    }
    if count > MAXIMUM_GALLERY_PICTURES {
        return Err(GalleryError::TooMany { count });
    }
    let sides_fit = |side: u32| side > 0 && side <= MAXIMUM_TILE_SIDE_PX;
    if !sides_fit(spec.tile_width_px)
        || !sides_fit(spec.tile_height_px)
        || spec.gap_px > MAXIMUM_TILE_SIDE_PX
        || spec.columns == Some(0)
    {
        return Err(GalleryError::InvalidSpec);
    }
    let count = u32::try_from(count).map_err(|_| GalleryError::TooLarge)?;
    let columns = spec.columns.unwrap_or_else(|| {
        // The fewest columns whose square is at least the count times the tile's height over its
        // width, which makes the whole sheet near square.
        let (width, height) = (
            u64::from(spec.tile_width_px),
            u64::from(spec.tile_height_px),
        );
        (1..count)
            .find(|columns| u64::from(*columns).pow(2) * width >= u64::from(count) * height)
            .unwrap_or(count)
    });
    let columns = columns.min(count);
    Ok((columns, count.div_ceil(columns)))
}

/// The sheet's width and height in pixels for a grid of `columns` by `rows` under `spec`.
fn sheet_size(columns: u32, rows: u32, spec: &GallerySpec) -> Result<(u32, u32), GalleryError> {
    let side = |cells: u32, tile: u32| {
        u64::from(cells) * u64::from(tile) + u64::from(cells + 1) * u64::from(spec.gap_px)
    };
    let (width, height) = (
        side(columns, spec.tile_width_px),
        side(rows, spec.tile_height_px),
    );
    let width = u32::try_from(width).map_err(|_| GalleryError::TooLarge)?;
    let height = u32::try_from(height).map_err(|_| GalleryError::TooLarge)?;
    if u64::from(width) * u64::from(height) > MAXIMUM_OUTPUT_PIXELS {
        return Err(GalleryError::TooLarge);
    }
    Ok((width, height))
}

/// One PNG that tiles `pictures` (encoded PNG, JPEG or WebP bytes) in order. Nothing is drawn but
/// the pictures: no labels, no text.
pub fn compose_gallery<P: AsRef<[u8]>>(
    pictures: &[P],
    spec: &GallerySpec,
) -> Result<Vec<u8>, GalleryError> {
    let (columns, rows) = gallery_grid(pictures.len(), spec)?;
    let (width, height) = sheet_size(columns, rows, spec)?;
    let mut sheet = RgbImage::from_pixel(width, height, Rgb(spec.background));
    for (index, bytes) in pictures.iter().enumerate() {
        let picture =
            image::load_from_memory(bytes.as_ref()).map_err(|_| GalleryError::Picture { index })?;
        let fitted = picture
            .resize(
                spec.tile_width_px,
                spec.tile_height_px,
                FilterType::Triangle,
            )
            .to_rgb8();
        let (fitted_width, fitted_height) = fitted.dimensions();
        let cell = u32::try_from(index).map_err(|_| GalleryError::TooLarge)?;
        let (column, row) = (cell % columns, cell / columns);
        let left = spec.gap_px
            + column * (spec.tile_width_px + spec.gap_px)
            + (spec.tile_width_px - fitted_width) / 2;
        let top = spec.gap_px
            + row * (spec.tile_height_px + spec.gap_px)
            + (spec.tile_height_px - fitted_height) / 2;
        image::imageops::replace(&mut sheet, &fitted, i64::from(left), i64::from(top));
    }
    let mut out = Cursor::new(Vec::new());
    DynamicImage::ImageRgb8(sheet)
        .write_to(&mut out, ImageFormat::Png)
        .map_err(|_| GalleryError::Encode)?;
    Ok(out.into_inner())
}
