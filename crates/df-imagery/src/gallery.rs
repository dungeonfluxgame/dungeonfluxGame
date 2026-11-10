//! A gallery sheet: many pictures tiled into one, so a backend that takes only a few reference
//! pictures can be shown a whole collection as a single picture. It is pure pixels: the pictures
//! come in as encoded bytes, in the order they are laid out, and one PNG comes out. It draws no
//! labels and no text, makes no call and reads no file.
//!
//! Each picture is scaled to fit its tile whole (its shape kept) and centred on the background. The
//! tile follows the pictures' shape unless the spec fixes it: its shape is that of the median
//! picture, so square paintings sit in square tiles and wide ones in wide tiles. The columns derive
//! from the count so that the sheet comes out roughly square, unless the spec fixes them; tiles
//! fill left to right, top to bottom, and a last row that is not full leaves its empty cells as
//! background.

use std::cmp::Ordering;
use std::io::Cursor;

use image::imageops::FilterType;
use image::{DynamicImage, GenericImageView, ImageFormat, Rgb, RgbImage};
use serde::{Deserialize, Serialize};

use crate::output::MAXIMUM_OUTPUT_PIXELS;

/// The most pictures one sheet holds.
pub const MAXIMUM_GALLERY_PICTURES: usize = 256;
/// The widest or tallest a tile or the gap may be, in pixels.
pub const MAXIMUM_TILE_SIDE_PX: u32 = 4_096;

/// How big and what shape each tile of a gallery is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GalleryTile {
    /// The shape of the median picture, its longer side `long_side_px`.
    Follow { long_side_px: u32 },
    /// This size for every tile.
    Fixed { width_px: u32, height_px: u32 },
}

/// How a gallery sheet is laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GallerySpec {
    /// The columns; derived from the count when none, so the sheet is roughly square.
    pub columns: Option<u32>,
    pub tile: GalleryTile,
    /// The space between tiles and round the edge.
    pub gap_px: u32,
    /// The colour of the sheet behind and between the tiles: `[red, green, blue]`.
    pub background: [u8; 3],
}

impl Default for GallerySpec {
    /// Tiles in the shape of the pictures, 448 px on the long side, a 12 px gap, mid-grey.
    fn default() -> Self {
        Self {
            columns: None,
            tile: GalleryTile::Follow { long_side_px: 448 },
            gap_px: 12,
            background: [128, 128, 128],
        }
    }
}

/// The grid a gallery is laid out in, in tiles and in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GalleryLayout {
    pub columns: u32,
    pub rows: u32,
    pub tile_width_px: u32,
    pub tile_height_px: u32,
    pub width_px: u32,
    pub height_px: u32,
}

/// A gallery sheet: the PNG and the grid it was laid out in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gallery {
    pub png: Vec<u8>,
    pub layout: GalleryLayout,
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

/// Whether the shape `a` (width, height) is wider than `b`, by cross-multiplication.
fn by_aspect(a: &(u32, u32), b: &(u32, u32)) -> Ordering {
    (u64::from(a.0) * u64::from(b.1)).cmp(&(u64::from(b.0) * u64::from(a.1)))
}

/// The tile of `spec` for pictures of `shapes` (width, height).
fn tile_for(shapes: &[(u32, u32)], tile: GalleryTile) -> Result<(u32, u32), GalleryError> {
    let sides_fit = |side: u32| side > 0 && side <= MAXIMUM_TILE_SIDE_PX;
    match tile {
        GalleryTile::Fixed {
            width_px,
            height_px,
        } => {
            if sides_fit(width_px) && sides_fit(height_px) {
                Ok((width_px, height_px))
            } else {
                Err(GalleryError::InvalidSpec)
            }
        }
        GalleryTile::Follow { long_side_px } => {
            if !sides_fit(long_side_px) {
                return Err(GalleryError::InvalidSpec);
            }
            let mut sorted = shapes.to_vec();
            sorted.sort_by(by_aspect);
            let (width, height) = sorted[(sorted.len() - 1) / 2];
            let long = u64::from(long_side_px);
            let short = |own: u32, other: u32| {
                u32::try_from((long * u64::from(own) / u64::from(other)).max(1))
                    .unwrap_or(long_side_px)
            };
            Ok(if width >= height {
                (long_side_px, short(height, width))
            } else {
                (short(width, height), long_side_px)
            })
        }
    }
}

/// The grid `shapes` (the width and height of each picture, in the order they are laid out) are
/// laid out in under `spec`.
pub fn gallery_layout(
    shapes: &[(u32, u32)],
    spec: &GallerySpec,
) -> Result<GalleryLayout, GalleryError> {
    let count = shapes.len();
    if count == 0 {
        return Err(GalleryError::Empty);
    }
    if count > MAXIMUM_GALLERY_PICTURES {
        return Err(GalleryError::TooMany { count });
    }
    if spec.gap_px > MAXIMUM_TILE_SIDE_PX || spec.columns == Some(0) {
        return Err(GalleryError::InvalidSpec);
    }
    let (tile_width, tile_height) = tile_for(shapes, spec.tile)?;
    let count = u32::try_from(count).map_err(|_| GalleryError::TooLarge)?;
    let columns = spec
        .columns
        .unwrap_or_else(|| {
            // The fewest columns whose square is at least the count times the tile's height over
            // its width, which makes the whole sheet near square.
            let (width, height) = (u64::from(tile_width), u64::from(tile_height));
            (1..count)
                .find(|columns| u64::from(*columns).pow(2) * width >= u64::from(count) * height)
                .unwrap_or(count)
        })
        .min(count);
    let rows = count.div_ceil(columns);
    let side = |cells: u32, tile: u32| {
        u64::from(cells) * u64::from(tile) + u64::from(cells + 1) * u64::from(spec.gap_px)
    };
    let width = u32::try_from(side(columns, tile_width)).map_err(|_| GalleryError::TooLarge)?;
    let height = u32::try_from(side(rows, tile_height)).map_err(|_| GalleryError::TooLarge)?;
    if u64::from(width) * u64::from(height) > MAXIMUM_OUTPUT_PIXELS {
        return Err(GalleryError::TooLarge);
    }
    Ok(GalleryLayout {
        columns,
        rows,
        tile_width_px: tile_width,
        tile_height_px: tile_height,
        width_px: width,
        height_px: height,
    })
}

/// One PNG that tiles `pictures` (encoded PNG, JPEG or WebP bytes) in order, and the grid it was
/// laid out in. Nothing is drawn but the pictures: no labels, no text.
pub fn compose_gallery<P: AsRef<[u8]>>(
    pictures: &[P],
    spec: &GallerySpec,
) -> Result<Gallery, GalleryError> {
    if pictures.is_empty() {
        return Err(GalleryError::Empty);
    }
    if pictures.len() > MAXIMUM_GALLERY_PICTURES {
        return Err(GalleryError::TooMany {
            count: pictures.len(),
        });
    }
    let decoded = pictures
        .iter()
        .enumerate()
        .map(|(index, bytes)| {
            image::load_from_memory(bytes.as_ref()).map_err(|_| GalleryError::Picture { index })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let shapes: Vec<(u32, u32)> = decoded.iter().map(GenericImageView::dimensions).collect();
    let layout = gallery_layout(&shapes, spec)?;
    let mut sheet = RgbImage::from_pixel(layout.width_px, layout.height_px, Rgb(spec.background));
    for (index, picture) in decoded.iter().enumerate() {
        let fitted = picture
            .resize(
                layout.tile_width_px,
                layout.tile_height_px,
                FilterType::Triangle,
            )
            .to_rgb8();
        let (fitted_width, fitted_height) = fitted.dimensions();
        let cell = u32::try_from(index).map_err(|_| GalleryError::TooLarge)?;
        let (column, row) = (cell % layout.columns, cell / layout.columns);
        let left = spec.gap_px
            + column * (layout.tile_width_px + spec.gap_px)
            + (layout.tile_width_px - fitted_width) / 2;
        let top = spec.gap_px
            + row * (layout.tile_height_px + spec.gap_px)
            + (layout.tile_height_px - fitted_height) / 2;
        image::imageops::replace(&mut sheet, &fitted, i64::from(left), i64::from(top));
    }
    let mut out = Cursor::new(Vec::new());
    DynamicImage::ImageRgb8(sheet)
        .write_to(&mut out, ImageFormat::Png)
        .map_err(|_| GalleryError::Encode)?;
    Ok(Gallery {
        png: out.into_inner(),
        layout,
    })
}
