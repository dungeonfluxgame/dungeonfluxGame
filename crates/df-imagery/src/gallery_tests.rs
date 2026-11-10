//! Tests of the gallery sheet: its grid, its tiles and its refusals.

use std::io::Cursor;

use image::{DynamicImage, ImageFormat, Rgb, RgbImage};

use crate::gallery::{
    GalleryError, GallerySpec, MAXIMUM_GALLERY_PICTURES, compose_gallery, gallery_grid,
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
        tile_width_px: 40,
        tile_height_px: 30,
        gap_px: 4,
        background: [128, 128, 128],
    }
}

fn sheet_of(bytes: &[u8]) -> RgbImage {
    image::load_from_memory(bytes).unwrap().to_rgb8()
}

#[test]
fn columns_derive_from_the_count_so_the_sheet_is_near_square() {
    let spec = GallerySpec::default();
    assert_eq!(gallery_grid(1, &spec), Ok((1, 1)));
    assert_eq!(gallery_grid(4, &spec), Ok((2, 2)));
    assert_eq!(gallery_grid(20, &spec), Ok((4, 5)));
    assert_eq!(gallery_grid(64, &spec), Ok((6, 11)));
    let fixed = GallerySpec {
        columns: Some(5),
        ..spec
    };
    assert_eq!(gallery_grid(20, &fixed), Ok((5, 4)));
    let wide = GallerySpec {
        columns: Some(9),
        ..spec
    };
    assert_eq!(gallery_grid(3, &wide), Ok((3, 1)));
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
    let sheet = sheet_of(&compose_gallery(&[red, green, blue], &spec).unwrap());
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
fn a_picture_keeps_its_shape_and_is_centred_in_its_tile() {
    let tall = solid(10, 30, [200, 100, 50], ImageFormat::Png);
    let spec = GallerySpec {
        columns: Some(1),
        ..small_spec()
    };
    let sheet = sheet_of(&compose_gallery(&[tall], &spec).unwrap());
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
    let sheet = sheet_of(&compose_gallery(&[big], &spec).unwrap());
    assert_eq!(sheet.dimensions(), (40 + 8, 30 + 8));
    assert_eq!(sheet.get_pixel(4, 4), &Rgb([10, 20, 30]));
    assert_eq!(sheet.get_pixel(4 + 39, 4 + 29), &Rgb([10, 20, 30]));
}

#[test]
fn jpeg_and_png_pictures_are_read_together() {
    let jpeg = solid(40, 30, [90, 90, 90], ImageFormat::Jpeg);
    let png = solid(40, 30, [90, 90, 90], ImageFormat::Png);
    let sheet = compose_gallery(&[jpeg, png], &small_spec()).unwrap();
    assert!(!sheet.is_empty());
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
    for bad in [
        GallerySpec {
            tile_width_px: 0,
            ..spec
        },
        GallerySpec {
            tile_height_px: 0,
            ..spec
        },
        GallerySpec {
            columns: Some(0),
            ..spec
        },
        GallerySpec {
            tile_width_px: 5_000,
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
        tile_width_px: 4_096,
        tile_height_px: 4_096,
        gap_px: 0,
        background: [0, 0, 0],
    };
    assert_eq!(
        compose_gallery(&vec![one; 64], &huge),
        Err(GalleryError::TooLarge)
    );
}
