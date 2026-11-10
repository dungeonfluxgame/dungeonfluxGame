//! Tests of the gallery sheet: its grid, its tiles and its refusals.

use std::io::Cursor;

use image::{DynamicImage, ImageFormat, Rgb, RgbImage};

use crate::gallery::{
    GalleryError, GalleryLayout, GallerySpec, GalleryTile, MAXIMUM_GALLERY_PICTURES,
    compose_gallery, gallery_layout,
};

fn solid(width: u32, height: u32, color: [u8; 3], format: ImageFormat) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    DynamicImage::ImageRgb8(RgbImage::from_pixel(width, height, Rgb(color)))
        .write_to(&mut out, format)
        .unwrap();
    out.into_inner()
}

fn small_spec() -> GallerySpec {
    GallerySpec {
        columns: None,
        tile: GalleryTile::Fixed {
            width_px: 40,
            height_px: 30,
        },
        gap_px: 4,
        background: [128, 128, 128],
    }
}

fn sheet_of(bytes: &[u8]) -> RgbImage {
    image::load_from_memory(bytes).unwrap().to_rgb8()
}

fn layout_of(shape: (u32, u32), count: usize, spec: &GallerySpec) -> GalleryLayout {
    gallery_layout(&vec![shape; count], spec).unwrap()
}

#[test]
fn square_pictures_get_square_tiles_and_a_near_square_sheet() {
    let spec = GallerySpec::default();
    let one = layout_of((512, 512), 1, &spec);
    assert_eq!((one.columns, one.rows), (1, 1));
    assert_eq!((one.tile_width_px, one.tile_height_px), (448, 448));
    let twenty = layout_of((512, 512), 20, &spec);
    assert_eq!((twenty.columns, twenty.rows), (5, 4));
    assert_eq!((twenty.tile_width_px, twenty.tile_height_px), (448, 448));
    assert_eq!(twenty.width_px, 5 * 448 + 6 * 12);
    assert_eq!(twenty.height_px, 4 * 448 + 5 * 12);
}

#[test]
fn wide_pictures_get_wide_tiles() {
    let spec = GallerySpec::default();
    let twenty = layout_of((1280, 720), 20, &spec);
    assert_eq!((twenty.tile_width_px, twenty.tile_height_px), (448, 252));
    assert_eq!((twenty.columns, twenty.rows), (4, 5));
    let tall = layout_of((300, 600), 4, &spec);
    assert_eq!((tall.tile_width_px, tall.tile_height_px), (224, 448));
    assert_eq!((tall.columns, tall.rows), (3, 2));
}

#[test]
fn the_tile_follows_the_median_picture() {
    let spec = GallerySpec::default();
    let shapes = [(100, 100), (160, 90), (100, 100), (100, 100), (90, 160)];
    let layout = gallery_layout(&shapes, &spec).unwrap();
    assert_eq!((layout.tile_width_px, layout.tile_height_px), (448, 448));
    let mostly_wide = [(160, 90), (160, 90), (100, 100)];
    let layout = gallery_layout(&mostly_wide, &spec).unwrap();
    assert_eq!((layout.tile_width_px, layout.tile_height_px), (448, 252));
}

#[test]
fn columns_can_be_fixed_and_a_fixed_tile_ignores_the_pictures() {
    let fixed = GallerySpec {
        columns: Some(5),
        ..GallerySpec::default()
    };
    let layout = layout_of((512, 512), 20, &fixed);
    assert_eq!((layout.columns, layout.rows), (5, 4));
    let wide = GallerySpec {
        columns: Some(9),
        ..GallerySpec::default()
    };
    assert_eq!(layout_of((512, 512), 3, &wide).columns, 3);
    let layout = layout_of((512, 512), 4, &small_spec());
    assert_eq!((layout.tile_width_px, layout.tile_height_px), (40, 30));
}

#[test]
fn tiles_fill_left_to_right_then_down_on_the_background() {
    let red = solid(40, 30, [255, 0, 0], ImageFormat::Png);
    let green = solid(40, 30, [0, 255, 0], ImageFormat::Png);
    let blue = solid(40, 30, [0, 0, 255], ImageFormat::Png);
    let spec = GallerySpec {
        columns: Some(2),
        ..small_spec()
    };
    let made = compose_gallery(&[red, green, blue], &spec).unwrap();
    assert_eq!((made.layout.columns, made.layout.rows), (2, 2));
    let sheet = sheet_of(&made.png);
    assert_eq!(sheet.dimensions(), (2 * 40 + 3 * 4, 2 * 30 + 3 * 4));
    assert_eq!(sheet.get_pixel(4 + 20, 4 + 15), &Rgb([255, 0, 0]));
    assert_eq!(sheet.get_pixel(4 + 40 + 4 + 20, 4 + 15), &Rgb([0, 255, 0]));
    assert_eq!(sheet.get_pixel(4 + 20, 4 + 30 + 4 + 15), &Rgb([0, 0, 255]));
    // The gap, the edge and the empty fourth cell are the background.
    assert_eq!(sheet.get_pixel(0, 0), &Rgb([128, 128, 128]));
    assert_eq!(sheet.get_pixel(4 + 40 + 1, 10), &Rgb([128, 128, 128]));
    assert_eq!(
        sheet.get_pixel(4 + 40 + 4 + 20, 4 + 30 + 4 + 15),
        &Rgb([128, 128, 128])
    );
}

#[test]
fn square_members_fill_their_square_tiles() {
    let spec = GallerySpec {
        tile: GalleryTile::Follow { long_side_px: 40 },
        columns: Some(2),
        ..small_spec()
    };
    let pictures: Vec<Vec<u8>> = (0..4)
        .map(|n| solid(100, 100, [10 * n + 10, 0, 0], ImageFormat::Png))
        .collect();
    let made = compose_gallery(&pictures, &spec).unwrap();
    assert_eq!(
        (made.layout.tile_width_px, made.layout.tile_height_px),
        (40, 40)
    );
    let sheet = sheet_of(&made.png);
    assert_eq!(sheet.dimensions(), (2 * 40 + 3 * 4, 2 * 40 + 3 * 4));
    // The corners of a tile are the picture, not background: the picture fills the tile.
    assert_eq!(sheet.get_pixel(4, 4), &Rgb([10, 0, 0]));
    assert_eq!(sheet.get_pixel(4 + 39, 4 + 39), &Rgb([10, 0, 0]));
    assert_eq!(sheet.get_pixel(4 + 40 + 4, 4), &Rgb([20, 0, 0]));
}

#[test]
fn a_picture_keeps_its_shape_and_is_centred_in_its_tile() {
    let tall = solid(10, 30, [200, 100, 50], ImageFormat::Png);
    let spec = GallerySpec {
        columns: Some(1),
        ..small_spec()
    };
    let sheet = sheet_of(&compose_gallery(&[tall], &spec).unwrap().png);
    // 10 by 30 fits a 40 by 30 tile whole at its own size, with 15 px of background either side.
    assert_eq!(sheet.get_pixel(4 + 15 + 5, 4 + 15), &Rgb([200, 100, 50]));
    assert_eq!(sheet.get_pixel(4 + 5, 4 + 15), &Rgb([128, 128, 128]));
    assert_eq!(sheet.get_pixel(4 + 40 - 5, 4 + 15), &Rgb([128, 128, 128]));
}

#[test]
fn a_larger_picture_is_scaled_down_to_its_tile() {
    let big = solid(400, 300, [10, 20, 30], ImageFormat::Png);
    let spec = GallerySpec {
        columns: Some(1),
        ..small_spec()
    };
    let sheet = sheet_of(&compose_gallery(&[big], &spec).unwrap().png);
    assert_eq!(sheet.dimensions(), (40 + 8, 30 + 8));
    assert_eq!(sheet.get_pixel(4, 4), &Rgb([10, 20, 30]));
    assert_eq!(sheet.get_pixel(4 + 39, 4 + 29), &Rgb([10, 20, 30]));
}

#[test]
fn jpeg_and_png_pictures_are_read_together() {
    let jpeg = solid(40, 30, [90, 90, 90], ImageFormat::Jpeg);
    let png = solid(40, 30, [90, 90, 90], ImageFormat::Png);
    let made = compose_gallery(&[jpeg, png], &small_spec()).unwrap();
    assert!(!made.png.is_empty());
}

#[test]
fn the_same_pictures_give_the_same_sheet() {
    let pictures = [
        solid(40, 30, [1, 2, 3], ImageFormat::Png),
        solid(20, 30, [4, 5, 6], ImageFormat::Png),
    ];
    assert_eq!(
        compose_gallery(&pictures, &small_spec()),
        compose_gallery(&pictures, &small_spec())
    );
}

#[test]
fn a_gallery_of_nothing_or_too_many_is_refused() {
    let none: [Vec<u8>; 0] = [];
    assert_eq!(
        compose_gallery(&none, &small_spec()),
        Err(GalleryError::Empty)
    );
    let one = solid(4, 4, [1, 1, 1], ImageFormat::Png);
    let many = vec![one; MAXIMUM_GALLERY_PICTURES + 1];
    assert_eq!(
        compose_gallery(&many, &small_spec()),
        Err(GalleryError::TooMany {
            count: MAXIMUM_GALLERY_PICTURES + 1
        })
    );
}

#[test]
fn a_picture_that_does_not_decode_names_its_place() {
    let good = solid(4, 4, [1, 1, 1], ImageFormat::Png);
    assert_eq!(
        compose_gallery(&[good, b"not a picture".to_vec()], &small_spec()),
        Err(GalleryError::Picture { index: 1 })
    );
}

#[test]
fn a_spec_that_cannot_be_laid_out_is_refused() {
    let one = [solid(4, 4, [1, 1, 1], ImageFormat::Png)];
    let spec = small_spec();
    let tile = |width_px, height_px| GalleryTile::Fixed {
        width_px,
        height_px,
    };
    for bad in [
        GallerySpec {
            tile: tile(0, 30),
            ..spec
        },
        GallerySpec {
            tile: tile(40, 0),
            ..spec
        },
        GallerySpec {
            tile: tile(5_000, 30),
            ..spec
        },
        GallerySpec {
            tile: GalleryTile::Follow { long_side_px: 0 },
            ..spec
        },
        GallerySpec {
            columns: Some(0),
            ..spec
        },
        GallerySpec {
            gap_px: 5_000,
            ..spec
        },
    ] {
        assert_eq!(compose_gallery(&one, &bad), Err(GalleryError::InvalidSpec));
    }
}

#[test]
fn a_sheet_past_the_pixel_bound_is_refused() {
    let one = solid(4, 4, [1, 1, 1], ImageFormat::Png);
    let huge = GallerySpec {
        columns: Some(16),
        tile: GalleryTile::Fixed {
            width_px: 4_096,
            height_px: 4_096,
        },
        gap_px: 0,
        background: [0, 0, 0],
    };
    assert_eq!(
        compose_gallery(&vec![one; 64], &huge),
        Err(GalleryError::TooLarge)
    );
}
