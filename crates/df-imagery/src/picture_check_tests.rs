//! Tests of the near-blank picture check.

use std::io::Cursor;

use image::{DynamicImage, ImageFormat, Rgb, RgbImage};

use crate::picture_check::{PictureCheckError, measure_picture};

fn png(picture: RgbImage) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    DynamicImage::ImageRgb8(picture)
        .write_to(&mut out, ImageFormat::Png)
        .unwrap();
    out.into_inner()
}

fn board(width: u32, height: u32) -> RgbImage {
    RgbImage::from_pixel(width, height, Rgb([232, 224, 196]))
}

/// A little deterministic noise, so a blank is not perfectly flat.
fn noisy(mut picture: RgbImage, amplitude: u8) -> RgbImage {
    let mut state = 12_345_u32;
    for pixel in picture.pixels_mut() {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let jitter = u8::try_from((state >> 24) % (u32::from(amplitude) * 2 + 1)).unwrap();
        for channel in &mut pixel.0 {
            *channel = channel.saturating_add(jitter).saturating_sub(amplitude);
        }
    }
    picture
}

#[test]
fn a_flat_board_is_blank() {
    let stats = measure_picture(&png(board(640, 360))).unwrap();
    assert!(stats.is_near_blank(), "{stats:?}");
}

#[test]
fn a_board_with_a_pale_edge_and_a_little_noise_is_blank() {
    let mut picture = noisy(board(640, 360), 2);
    for x in 0..640 {
        for y in (0..30).chain(330..360) {
            picture.put_pixel(x, y, Rgb([250, 248, 240]));
        }
    }
    for y in 0..360 {
        for x in (0..30).chain(610..640) {
            picture.put_pixel(x, y, Rgb([250, 248, 240]));
        }
    }
    let stats = measure_picture(&png(picture)).unwrap();
    assert!(stats.is_near_blank(), "{stats:?}");
}

#[test]
fn a_picture_with_a_sky_and_a_ground_is_not_blank() {
    let mut picture = RgbImage::from_pixel(640, 360, Rgb([150, 170, 200]));
    for y in 200..360 {
        for x in 0..640 {
            picture.put_pixel(x, y, Rgb([90, 80, 50]));
        }
    }
    let stats = measure_picture(&png(picture)).unwrap();
    assert!(!stats.is_near_blank(), "{stats:?}");
}

#[test]
fn a_small_painted_shape_on_the_board_is_not_blank() {
    let mut picture = board(640, 360);
    for y in 150..210 {
        for x in 290..350 {
            picture.put_pixel(x, y, Rgb([120, 60, 40]));
        }
    }
    let stats = measure_picture(&png(picture)).unwrap();
    assert!(!stats.is_near_blank(), "{stats:?}");
}

#[test]
fn fine_brushwork_is_not_blank() {
    let mut picture = board(640, 360);
    for y in 0..360 {
        for x in 0..640 {
            if (x / 30 + y / 30) % 2 == 0 {
                picture.put_pixel(x, y, Rgb([205, 190, 150]));
            }
        }
    }
    let stats = measure_picture(&png(picture)).unwrap();
    assert!(!stats.is_near_blank(), "{stats:?}");
}

#[test]
fn a_picture_that_does_not_decode_is_an_error() {
    assert_eq!(measure_picture(b"nothing"), Err(PictureCheckError));
}
