//! Analytical pixel fixtures for the private raster executor.
//!
//! Tiny images with known colors pin order, geometry, color, mask, alpha,
//! limit, corruption, and encoding behavior without providers or files.

use std::cell::Cell;
use std::collections::HashMap;
use std::io::Cursor;

use image::{DynamicImage, RgbImage, RgbaImage};
use sha2::{Digest, Sha256};

use crate::config::TransformLimits;
use crate::output::{AlphaHandling, Dimensions, Encoding, LossyQuality, RasterOutputSpec, Rgb};
use crate::raster::{RasterError, execute_transform};
use crate::request::{AssetDigest, RequestLimits, SourceRef};
use crate::transform::{
    AlignAnchor, Anchor1D, BlendMode, BlurAlgorithm, BlurSpec, BrightnessOffset, ColorEffect,
    CompositeLayer, CoverageRef, ExposureStops, FlipAxis, ImageTransform, Interpolation,
    MaskChannel, MaskMode,
    MaskSource, MaskSpec, PadSpec, PixelRect, ResizeMode, ResizeSpec, RotateSpec, SharpenSpec,
    TransformSource, TransformSpec,
};
use df_assets::{AssetId, Audience};

fn digest_of(bytes: &[u8]) -> AssetDigest {
    AssetDigest::from_bytes(Sha256::digest(bytes).into())
}

fn asset_byte(byte: u8) -> AssetId {
    AssetId::from_bytes(&[byte; 16]).unwrap()
}

fn pin(byte: u8, bytes: &[u8]) -> SourceRef {
    SourceRef {
        asset: asset_byte(byte),
        digest: digest_of(bytes),
        audience: Audience::PublicStatic,
    }
}

fn wrong_digest_pin(byte: u8) -> SourceRef {
    SourceRef {
        asset: asset_byte(byte),
        digest: AssetDigest::from_bytes([0x00; 32]),
        audience: Audience::PublicStatic,
    }
}

fn resolver_for(
    map: &HashMap<[u8; 16], Vec<u8>>,
) -> impl Fn(&SourceRef) -> Result<Vec<u8>, RasterError> + '_ {
    move |source| {
        map.get(source.asset.as_bytes())
            .cloned()
            .ok_or(RasterError::ResolveFailed)
    }
}

fn allow() -> impl Fn() -> Result<(), RasterError> {
    || Ok(())
}

fn limits() -> (RequestLimits, TransformLimits) {
    (
        RequestLimits::default(),
        TransformLimits {
            enabled: true,
            max_concurrency: 2,
            max_decoded_bytes: 64 * 1024 * 1024,
            max_output_bytes: 16 * 1024 * 1024,
            queue_capacity: 64,
            deadline_secs: 30,
        },
    )
}

fn rgba_fixture(width: u32, height: u32, paint: impl Fn(u32, u32) -> [u8; 4]) -> RgbaImage {
    RgbaImage::from_fn(width, height, |x, y| image::Rgba(paint(x, y)))
}

fn png_bytes(image: &RgbaImage) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(image.clone())
        .write_to(&mut out, image::ImageFormat::Png)
        .unwrap();
    out.into_inner()
}

fn jpeg_bytes(image: &RgbImage, quality: u8) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality)
        .encode_image(image)
        .unwrap();
    out.into_inner()
}

fn run_transform(
    spec: &TransformSpec,
    output: &RasterOutputSpec,
    map: &HashMap<[u8; 16], Vec<u8>>,
    request: &RequestLimits,
    transform: &TransformLimits,
) -> Result<crate::raster::RasterOutput, RasterError> {
    execute_transform(spec, output, request, transform, &resolver_for(map), &allow())
}

fn png_output(width: u32, height: u32, alpha: AlphaHandling) -> RasterOutputSpec {
    RasterOutputSpec::new(Dimensions::new(width, height).unwrap(), alpha, Encoding::Png).unwrap()
}

fn recipe(
    byte: u8,
    bytes: &[u8],
    steps: Vec<ImageTransform>,
) -> (TransformSpec, HashMap<[u8; 16], Vec<u8>>) {
    let mut map = HashMap::new();
    map.insert([byte; 16], bytes.to_vec());
    (
        TransformSpec {
            source: TransformSource::Image(pin(byte, bytes)),
            steps,
        },
        map,
    )
}

fn decode_output(bytes: &[u8]) -> RgbaImage {
    image::load_from_memory(bytes).unwrap().to_rgba8()
}

#[test]
fn color_revision_is_pinned_for_recipe_keys() {
    assert_eq!(crate::raster::color::RASTER_COLOR_REV, 2);
}

#[test]
fn corrupt_bytes_are_refused_as_decode_failed() {
    let bytes = b"\x89PNG\r\n\x1a\nnot-an-image".to_vec();
    let (spec, map) = recipe(0x11, &bytes, Vec::new());
    let (request, transform) = limits();
    let output = png_output(1, 1, AlphaHandling::Opaque);
    assert_eq!(
        run_transform(&spec, &output, &map, &request, &transform).unwrap_err(),
        RasterError::DecodeFailed
    );
}

#[test]
fn gif_bytes_are_refused_as_unsupported_format() {
    let bytes = b"GIF89a\x01\x00\x01\x00\x80\x00\x00".to_vec();
    let (spec, map) = recipe(0x12, &bytes, Vec::new());
    let (request, transform) = limits();
    let output = png_output(1, 1, AlphaHandling::Opaque);
    assert_eq!(
        run_transform(&spec, &output, &map, &request, &transform).unwrap_err(),
        RasterError::UnsupportedFormat
    );
}

#[test]
fn oversize_header_is_refused_before_allocation() {
    let tiny = rgba_fixture(1, 1, |_, _| [9, 9, 9, 255]);
    let mut bytes = png_bytes(&tiny);
    // Claim 5000x5000 in IHDR and repair its CRC so the header probe, not a
    // CRC failure, decides the outcome.
    bytes[16..20].copy_from_slice(&5000_u32.to_be_bytes());
    bytes[20..24].copy_from_slice(&5000_u32.to_be_bytes());
    let checksum = png_crc32(&bytes[12..29]);
    bytes[29..33].copy_from_slice(&checksum.to_be_bytes());
    let (spec, map) = recipe(0x13, &bytes, Vec::new());
    let (request, transform) = limits();
    let output = png_output(1, 1, AlphaHandling::Opaque);
    assert_eq!(
        run_transform(&spec, &output, &map, &request, &transform).unwrap_err(),
        RasterError::DecodedTooLarge {
            pixels: 25_000_000
        }
    );
}

#[test]
fn oversize_input_bytes_are_refused_before_decode() {
    let tiny = rgba_fixture(2, 2, |_, _| [9, 9, 9, 255]);
    let bytes = png_bytes(&tiny);
    let (spec, map) = recipe(0x14, &bytes, Vec::new());
    let (request, mut transform) = limits();
    transform.max_decoded_bytes = 16;
    let output = png_output(2, 2, AlphaHandling::Opaque);
    assert_eq!(
        run_transform(&spec, &output, &map, &request, &transform).unwrap_err(),
        RasterError::InputTooLarge {
            len_bytes: bytes.len() as u64
        }
    );
}

#[test]
fn digest_mismatch_is_refused_before_decode() {
    let tiny = rgba_fixture(2, 2, |_, _| [9, 9, 9, 255]);
    let bytes = png_bytes(&tiny);
    let mut map = HashMap::new();
    map.insert([0x15; 16], bytes.clone());
    let spec = TransformSpec {
        source: TransformSource::Image(wrong_digest_pin(0x15)),
        steps: Vec::new(),
    };
    let (request, transform) = limits();
    let output = png_output(2, 2, AlphaHandling::Opaque);
    assert_eq!(
        run_transform(&spec, &output, &map, &request, &transform).unwrap_err(),
        RasterError::DigestMismatch
    );
}

#[test]
fn resolver_failure_surfaces_without_work() {
    let spec = TransformSpec {
        source: TransformSource::Image(wrong_digest_pin(0x16)),
        steps: Vec::new(),
    };
    let map = HashMap::new();
    let (request, transform) = limits();
    let output = png_output(2, 2, AlphaHandling::Opaque);
    assert_eq!(
        run_transform(&spec, &output, &map, &request, &transform).unwrap_err(),
        RasterError::ResolveFailed
    );
}

#[test]
fn disabled_transforms_refuse_before_touching_bytes() {
    let reached = Cell::new(false);
    let tiny = rgba_fixture(2, 2, |_, _| [9, 9, 9, 255]);
    let bytes = png_bytes(&tiny);
    let source = pin(0x17, &bytes);
    let spec = TransformSpec {
        source: TransformSource::Image(source.clone()),
        steps: Vec::new(),
    };
    let (request, mut transform) = limits();
    transform.enabled = false;
    let output = png_output(2, 2, AlphaHandling::Opaque);
    let resolve = |_: &SourceRef| -> Result<Vec<u8>, RasterError> {
        reached.set(true);
        Ok(bytes.clone())
    };
    let outcome = execute_transform(&spec, &output, &request, &transform, &resolve, &allow());
    assert_eq!(outcome.unwrap_err(), RasterError::TransformsDisabled);
    assert!(!reached.get());
}

#[test]
fn too_many_steps_are_refused() {
    let tiny = rgba_fixture(2, 2, |_, _| [9, 9, 9, 255]);
    let bytes = png_bytes(&tiny);
    let flips = vec![ImageTransform::Flip(FlipAxis::Vertical); 33];
    let (spec, map) = recipe(0x18, &bytes, flips);
    let (request, transform) = limits();
    let output = png_output(2, 2, AlphaHandling::Opaque);
    assert_eq!(
        run_transform(&spec, &output, &map, &request, &transform).unwrap_err(),
        RasterError::InvalidRecipe(crate::transform::TransformError::TooManySteps)
    );
}

#[test]
fn cancelled_steps_stop_the_recipe() {
    let tiny = rgba_fixture(2, 2, |_, _| [9, 9, 9, 255]);
    let bytes = png_bytes(&tiny);
    let source = pin(0x19, &bytes);
    let spec = TransformSpec {
        source: TransformSource::Image(source),
        steps: vec![
            ImageTransform::Flip(FlipAxis::Vertical),
            ImageTransform::Flip(FlipAxis::Vertical),
            ImageTransform::Flip(FlipAxis::Vertical),
        ],
    };
    let mut map = HashMap::new();
    map.insert([0x19; 16], bytes);
    let (request, transform) = limits();
    let output = png_output(2, 2, AlphaHandling::Opaque);
    let calls = Cell::new(0_u32);
    let check = || -> Result<(), RasterError> {
        calls.set(calls.get() + 1);
        if calls.get() > 2 {
            return Err(RasterError::Cancelled);
        }
        Ok(())
    };
    let outcome = execute_transform(
        &spec,
        &output,
        &request,
        &transform,
        &resolver_for(&map),
        &check,
    );
    assert_eq!(outcome.unwrap_err(), RasterError::Cancelled);
}

fn png_crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0_u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & (0_u32.wrapping_sub(crc & 1)));
        }
    }
    !crc
}

fn quadrant_fixture() -> RgbaImage {
    rgba_fixture(4, 4, |x, y| match (x < 2, y < 2) {
        (true, true) => [255, 0, 0, 255],
        (false, true) => [0, 255, 0, 255],
        (true, false) => [0, 0, 255, 255],
        (false, false) => [255, 255, 255, 255],
    })
}

fn contain_step(width: u32, height: u32) -> ImageTransform {
    ImageTransform::Resize(
        ResizeSpec::new(
            width,
            height,
            ResizeMode::Contain,
            AlignAnchor::center(),
            Interpolation::Lanczos3,
            false,
            None,
        )
        .unwrap(),
    )
}

#[test]
fn crop_then_resize_differs_from_resize_then_crop() {
    let bytes = png_bytes(&quadrant_fixture());
    let first = vec![
        ImageTransform::Crop(PixelRect::new(0, 0, 4, 2).unwrap()),
        contain_step(2, 1),
    ];
    let second = vec![
        contain_step(2, 2),
        ImageTransform::Crop(PixelRect::new(0, 0, 2, 1).unwrap()),
    ];
    let (request, transform) = limits();
    let output = png_output(2, 1, AlphaHandling::Opaque);
    let (first_spec, first_map) = recipe(0x21, &bytes, first);
    let (second_spec, second_map) = recipe(0x21, &bytes, second);
    let first_out = run_transform(&first_spec, &output, &first_map, &request, &transform).unwrap();
    let second_out =
        run_transform(&second_spec, &output, &second_map, &request, &transform).unwrap();
    assert_ne!(first_out.bytes, second_out.bytes);
}

#[test]
fn contain_preserves_aspect_within_bounds() {
    let solid = rgba_fixture(4, 2, |_, _| [255, 0, 0, 255]);
    let bytes = png_bytes(&solid);
    let (spec, map) = recipe(0x22, &bytes, vec![contain_step(3, 3)]);
    let (request, transform) = limits();
    let output = png_output(3, 2, AlphaHandling::Opaque);
    let result = run_transform(&spec, &output, &map, &request, &transform).unwrap();
    assert_eq!((result.width_px, result.height_px), (3, 2));
    assert!(!result.has_alpha);
    let decoded = decode_output(&result.bytes);
    assert!(decoded.pixels().all(|pixel| pixel.0 == [255, 0, 0, 255]));
}

#[test]
fn cover_crops_from_the_start_anchor() {
    let halves = rgba_fixture(8, 4, |x, _| {
        if x < 4 {
            [255, 0, 0, 255]
        } else {
            [0, 0, 255, 255]
        }
    });
    let bytes = png_bytes(&halves);
    let cover = ImageTransform::Resize(
        ResizeSpec::new(
            2,
            2,
            ResizeMode::Cover,
            AlignAnchor {
                x: Anchor1D::Start,
                y: Anchor1D::Start,
            },
            Interpolation::Nearest,
            true,
            None,
        )
        .unwrap(),
    );
    let (spec, map) = recipe(0x23, &bytes, vec![cover]);
    let (request, transform) = limits();
    let output = png_output(2, 2, AlphaHandling::Opaque);
    let result = run_transform(&spec, &output, &map, &request, &transform).unwrap();
    let decoded = decode_output(&result.bytes);
    assert!(decoded.pixels().all(|pixel| pixel.0 == [255, 0, 0, 255]));
}

#[test]
fn cover_crops_from_the_end_anchor() {
    let halves = rgba_fixture(8, 4, |x, _| {
        if x < 4 {
            [255, 0, 0, 255]
        } else {
            [0, 0, 255, 255]
        }
    });
    let bytes = png_bytes(&halves);
    let cover = ImageTransform::Resize(
        ResizeSpec::new(
            2,
            2,
            ResizeMode::Cover,
            AlignAnchor {
                x: Anchor1D::End,
                y: Anchor1D::End,
            },
            Interpolation::Nearest,
            true,
            None,
        )
        .unwrap(),
    );
    let (spec, map) = recipe(0x24, &bytes, vec![cover]);
    let (request, transform) = limits();
    let output = png_output(2, 2, AlphaHandling::Opaque);
    let result = run_transform(&spec, &output, &map, &request, &transform).unwrap();
    let decoded = decode_output(&result.bytes);
    assert!(decoded.pixels().all(|pixel| pixel.0 == [0, 0, 255, 255]));
}

#[test]
fn stretch_changes_aspect_to_exact_bounds() {
    let solid = rgba_fixture(2, 2, |_, _| [0, 255, 0, 255]);
    let bytes = png_bytes(&solid);
    let stretch = ImageTransform::Resize(
        ResizeSpec::new(
            4,
            1,
            ResizeMode::Stretch,
            AlignAnchor::center(),
            Interpolation::Bilinear,
            true,
            None,
        )
        .unwrap(),
    );
    let (spec, map) = recipe(0x25, &bytes, vec![stretch]);
    let (request, transform) = limits();
    let output = png_output(4, 1, AlphaHandling::Opaque);
    let result = run_transform(&spec, &output, &map, &request, &transform).unwrap();
    assert_eq!((result.width_px, result.height_px), (4, 1));
    let decoded = decode_output(&result.bytes);
    assert!(decoded.pixels().all(|pixel| pixel.0 == [0, 255, 0, 255]));
}

#[test]
fn pad_places_content_on_the_fill_at_the_anchor() {
    let solid = rgba_fixture(2, 2, |_, _| [255, 0, 0, 255]);
    let bytes = png_bytes(&solid);
    let fill = Rgb {
        red: 1,
        green: 2,
        blue: 3,
    };
    let pad = ImageTransform::Pad(
        PadSpec::new(
            4,
            4,
            AlignAnchor {
                x: Anchor1D::Start,
                y: Anchor1D::Start,
            },
            fill,
        )
        .unwrap(),
    );
    let (spec, map) = recipe(0x26, &bytes, vec![pad]);
    let (request, transform) = limits();
    let output = png_output(4, 4, AlphaHandling::Opaque);
    let result = run_transform(&spec, &output, &map, &request, &transform).unwrap();
    let decoded = decode_output(&result.bytes);
    assert_eq!(decoded.get_pixel(0, 0).0, [255, 0, 0, 255]);
    assert_eq!(decoded.get_pixel(3, 3).0, [1, 2, 3, 255]);
}

#[test]
fn refused_upscale_leaves_small_content_unchanged() {
    let solid = rgba_fixture(2, 2, |_, _| [9, 9, 9, 255]);
    let bytes = png_bytes(&solid);
    let (spec, map) = recipe(0x27, &bytes, vec![contain_step(8, 8)]);
    let (request, transform) = limits();
    let output = png_output(2, 2, AlphaHandling::Opaque);
    let result = run_transform(&spec, &output, &map, &request, &transform).unwrap();
    assert_eq!((result.width_px, result.height_px), (2, 2));
    let decoded = decode_output(&result.bytes);
    assert!(decoded.pixels().all(|pixel| pixel.0 == [9, 9, 9, 255]));
}

#[test]
fn crop_outside_the_raster_is_refused() {
    let solid = rgba_fixture(2, 2, |_, _| [9, 9, 9, 255]);
    let bytes = png_bytes(&solid);
    let crop = ImageTransform::Crop(PixelRect::new(1, 1, 2, 2).unwrap());
    let (spec, map) = recipe(0x28, &bytes, vec![crop]);
    let (request, transform) = limits();
    let output = png_output(2, 2, AlphaHandling::Opaque);
    assert_eq!(
        run_transform(&spec, &output, &map, &request, &transform).unwrap_err(),
        RasterError::CropOutOfBounds
    );
}

#[test]
fn quarter_turn_rotates_pixel_positions() {
    let row = rgba_fixture(2, 1, |x, _| {
        if x == 0 {
            [255, 0, 0, 255]
        } else {
            [0, 0, 255, 255]
        }
    });
    let bytes = png_bytes(&row);
    let rotate = ImageTransform::Rotate(RotateSpec::QuarterTurns(1));
    let (spec, map) = recipe(0x29, &bytes, vec![rotate]);
    let (request, transform) = limits();
    let output = png_output(1, 2, AlphaHandling::Opaque);
    let result = run_transform(&spec, &output, &map, &request, &transform).unwrap();
    let decoded = decode_output(&result.bytes);
    assert_eq!(decoded.get_pixel(0, 0).0, [255, 0, 0, 255]);
    assert_eq!(decoded.get_pixel(0, 1).0, [0, 0, 255, 255]);
}

#[test]
fn flip_horizontal_mirrors_columns() {
    let row = rgba_fixture(2, 1, |x, _| {
        if x == 0 {
            [255, 0, 0, 255]
        } else {
            [0, 0, 255, 255]
        }
    });
    let bytes = png_bytes(&row);
    let flip = ImageTransform::Flip(FlipAxis::Horizontal);
    let (spec, map) = recipe(0x2a, &bytes, vec![flip]);
    let (request, transform) = limits();
    let output = png_output(2, 1, AlphaHandling::Opaque);
    let result = run_transform(&spec, &output, &map, &request, &transform).unwrap();
    let decoded = decode_output(&result.bytes);
    assert_eq!(decoded.get_pixel(0, 0).0, [0, 0, 255, 255]);
    assert_eq!(decoded.get_pixel(1, 0).0, [255, 0, 0, 255]);
}

#[test]
fn arbitrary_right_angle_matches_the_exact_quarter_turn() {
    let row = rgba_fixture(2, 1, |x, _| {
        if x == 0 {
            [255, 0, 0, 255]
        } else {
            [0, 0, 255, 255]
        }
    });
    let bytes = png_bytes(&row);
    let arbitrary = ImageTransform::Rotate(RotateSpec::Arbitrary {
        degrees: 90.0,
        expand_canvas: false,
        fill: Rgb {
            red: 0,
            green: 0,
            blue: 0,
        },
        interpolation: Interpolation::Bilinear,
    });
    let (spec, map) = recipe(0x2b, &bytes, vec![arbitrary]);
    let (request, transform) = limits();
    let output = png_output(1, 2, AlphaHandling::Opaque);
    let result = run_transform(&spec, &output, &map, &request, &transform).unwrap();
    let decoded = decode_output(&result.bytes);
    assert_eq!(decoded.get_pixel(0, 0).0, [255, 0, 0, 255]);
    assert_eq!(decoded.get_pixel(0, 1).0, [0, 0, 255, 255]);
}

#[test]
fn neutral_adjustments_leave_pixels_unchanged() {
    let base = rgba_fixture(2, 1, |x, _| {
        if x == 0 {
            [200, 100, 50, 255]
        } else {
            [10, 20, 30, 255]
        }
    });
    let bytes = png_bytes(&base);
    let steps = vec![
        ImageTransform::Brighten(BrightnessOffset::new(0.0).unwrap()),
        ImageTransform::Expose(ExposureStops::new(0.0).unwrap()),
        ImageTransform::Color(ColorEffect::contrast(1.0, 0.5).unwrap()),
        ImageTransform::Color(ColorEffect::saturation(1.0).unwrap()),
        ImageTransform::Color(ColorEffect::hue_shift(0.0).unwrap()),
        ImageTransform::Sharpen(SharpenSpec::new(1.0, 0.0, 10).unwrap()),
    ];
    let (spec, map) = recipe(0x31, &bytes, steps);
    let (request, transform) = limits();
    let output = png_output(2, 1, AlphaHandling::Opaque);
    let result = run_transform(&spec, &output, &map, &request, &transform).unwrap();
    let decoded = decode_output(&result.bytes);
    assert_eq!(decoded.get_pixel(0, 0).0, [200, 100, 50, 255]);
    assert_eq!(decoded.get_pixel(1, 0).0, [10, 20, 30, 255]);
}

#[test]
fn brightness_adds_offset_and_clamps() {
    let base = rgba_fixture(2, 1, |x, _| {
        if x == 0 {
            [0, 0, 0, 255]
        } else {
            [255, 255, 255, 255]
        }
    });
    let bytes = png_bytes(&base);
    let steps = vec![ImageTransform::Brighten(BrightnessOffset::new(0.5).unwrap())];
    let (spec, map) = recipe(0x32, &bytes, steps);
    let (request, transform) = limits();
    let output = png_output(2, 1, AlphaHandling::Opaque);
    let result = run_transform(&spec, &output, &map, &request, &transform).unwrap();
    let decoded = decode_output(&result.bytes);
    assert_eq!(decoded.get_pixel(0, 0).0, [128, 128, 128, 255]);
    assert_eq!(decoded.get_pixel(1, 0).0, [255, 255, 255, 255]);
}

#[test]
fn exposure_lift_keeps_black_and_white_pinned() {
    let base = rgba_fixture(3, 1, |x, _| match x {
        0 => [0, 0, 0, 255],
        1 => [128, 128, 128, 255],
        _ => [255, 255, 255, 255],
    });
    let bytes = png_bytes(&base);
    let steps = vec![ImageTransform::Expose(ExposureStops::new(1.0).unwrap())];
    let (spec, map) = recipe(0x33, &bytes, steps);
    let (request, transform) = limits();
    let output = png_output(3, 1, AlphaHandling::Opaque);
    let result = run_transform(&spec, &output, &map, &request, &transform).unwrap();
    let decoded = decode_output(&result.bytes);
    assert_eq!(decoded.get_pixel(0, 0).0, [0, 0, 0, 255]);
    let lifted = decoded.get_pixel(1, 0).0[0];
    assert!(lifted > 128 && lifted <= 180, "lifted={lifted}");
    assert_eq!(decoded.get_pixel(2, 0).0, [255, 255, 255, 255]);
}

#[test]
fn grayscale_uses_rec709_weights() {
    let base = rgba_fixture(3, 1, |x, _| match x {
        0 => [255, 0, 0, 255],
        1 => [0, 255, 0, 255],
        _ => [0, 0, 255, 255],
    });
    let bytes = png_bytes(&base);
    let steps = vec![ImageTransform::Color(ColorEffect::Grayscale)];
    let (spec, map) = recipe(0x34, &bytes, steps);
    let (request, transform) = limits();
    let output = png_output(3, 1, AlphaHandling::Opaque);
    let result = run_transform(&spec, &output, &map, &request, &transform).unwrap();
    let decoded = decode_output(&result.bytes);
    assert_eq!(decoded.get_pixel(0, 0).0, [54, 54, 54, 255]);
    assert_eq!(decoded.get_pixel(1, 0).0, [182, 182, 182, 255]);
    assert_eq!(decoded.get_pixel(2, 0).0, [18, 18, 18, 255]);
}

#[test]
fn saturation_zero_equals_grayscale() {
    let base = rgba_fixture(2, 1, |x, _| {
        if x == 0 {
            [200, 100, 50, 255]
        } else {
            [10, 200, 30, 255]
        }
    });
    let bytes = png_bytes(&base);
    let desaturate = vec![ImageTransform::Color(ColorEffect::saturation(0.0).unwrap())];
    let gray = vec![ImageTransform::Color(ColorEffect::Grayscale)];
    let (request, transform) = limits();
    let output = png_output(2, 1, AlphaHandling::Opaque);
    let (first_spec, first_map) = recipe(0x35, &bytes, desaturate);
    let (second_spec, second_map) = recipe(0x35, &bytes, gray);
    let first = run_transform(&first_spec, &output, &first_map, &request, &transform).unwrap();
    let second = run_transform(&second_spec, &output, &second_map, &request, &transform).unwrap();
    assert_eq!(first.bytes, second.bytes);
}

#[test]
fn hue_zero_is_the_identity() {
    let base = rgba_fixture(4, 1, |x, _| match x {
        0 => [255, 0, 0, 255],
        1 => [0, 255, 0, 255],
        2 => [0, 0, 255, 255],
        _ => [255, 255, 255, 255],
    });
    let bytes = png_bytes(&base);
    let steps = vec![ImageTransform::Color(ColorEffect::hue_shift(0.0).unwrap())];
    let (spec, map) = recipe(0x36, &bytes, steps);
    let (request, transform) = limits();
    let output = png_output(4, 1, AlphaHandling::Opaque);
    let result = run_transform(&spec, &output, &map, &request, &transform).unwrap();
    let decoded = decode_output(&result.bytes);
    for x in 0..4 {
        assert_eq!(decoded.get_pixel(x, 0).0, base.get_pixel(x, 0).0);
    }
}

#[test]
fn contrast_pivots_dark_toward_black() {
    let base = rgba_fixture(2, 1, |x, _| {
        if x == 0 {
            [32, 32, 32, 255]
        } else {
            [255, 255, 255, 255]
        }
    });
    let bytes = png_bytes(&base);
    let steps = vec![ImageTransform::Color(ColorEffect::contrast(2.0, 0.5).unwrap())];
    let (spec, map) = recipe(0x37, &bytes, steps);
    let (request, transform) = limits();
    let output = png_output(2, 1, AlphaHandling::Opaque);
    let result = run_transform(&spec, &output, &map, &request, &transform).unwrap();
    let decoded = decode_output(&result.bytes);
    assert_eq!(decoded.get_pixel(0, 0).0, [0, 0, 0, 255]);
    assert_eq!(decoded.get_pixel(1, 0).0, [255, 255, 255, 255]);
}

#[test]
fn gaussian_blur_spreads_a_single_bright_pixel() {
    let base = rgba_fixture(3, 1, |x, _| {
        if x == 1 {
            [255, 255, 255, 255]
        } else {
            [0, 0, 0, 255]
        }
    });
    let bytes = png_bytes(&base);
    let steps = vec![ImageTransform::Blur(BlurSpec::new(1.0, BlurAlgorithm::Gaussian).unwrap())];
    let (spec, map) = recipe(0x38, &bytes, steps);
    let (request, transform) = limits();
    let output = png_output(3, 1, AlphaHandling::Opaque);
    let result = run_transform(&spec, &output, &map, &request, &transform).unwrap();
    let decoded = decode_output(&result.bytes);
    assert!(decoded.get_pixel(1, 0).0[0] < 200);
    assert!(decoded.get_pixel(0, 0).0[0] > 0);
    assert!(decoded.get_pixel(2, 0).0[0] > 0);
}

#[allow(clippy::too_many_arguments, reason = "test fixture packs one mask recipe")]
fn mask_recipe(
    base_byte: u8,
    base: &RgbaImage,
    mask_byte: u8,
    mask: &RgbaImage,
    channel: MaskChannel,
    invert: bool,
    feather: f32,
    mode: MaskMode,
) -> (TransformSpec, HashMap<[u8; 16], Vec<u8>>) {
    let base_bytes = png_bytes(base);
    let mask_bytes = png_bytes(mask);
    let dimensions = Dimensions::new(mask.width(), mask.height()).unwrap();
    let mask_spec = MaskSpec::new(
        MaskSource::Asset(pin(mask_byte, &mask_bytes)),
        dimensions,
        AlignAnchor::center(),
        channel,
        invert,
        feather,
        mode,
    )
    .unwrap();
    let mut map = HashMap::new();
    map.insert([base_byte; 16], base_bytes.clone());
    map.insert([mask_byte; 16], mask_bytes.clone());
    let spec = TransformSpec {
        source: TransformSource::Image(pin(base_byte, &base_bytes)),
        steps: vec![ImageTransform::Mask(mask_spec)],
    };
    (spec, map)
}

#[test]
fn mask_multiply_applies_coverage_as_alpha() {
    let base = rgba_fixture(2, 1, |_, _| [255, 0, 0, 255]);
    let mask = rgba_fixture(2, 1, |x, _| {
        if x == 0 {
            [255, 255, 255, 0]
        } else {
            [255, 255, 255, 255]
        }
    });
    let (spec, map) = mask_recipe(
        0x41,
        &base,
        0x42,
        &mask,
        MaskChannel::Alpha,
        false,
        0.0,
        MaskMode::Multiply,
    );
    let (request, transform) = limits();
    let output = png_output(2, 1, AlphaHandling::PreserveAlpha);
    let result = run_transform(&spec, &output, &map, &request, &transform).unwrap();
    assert!(result.has_alpha);
    let decoded = decode_output(&result.bytes);
    assert_eq!(decoded.get_pixel(0, 0).0[3], 0);
    assert_eq!(decoded.get_pixel(1, 0).0[3], 255);
}

#[test]
fn mask_replace_writes_coverage_as_alpha() {
    let base = rgba_fixture(2, 1, |_, _| [255, 0, 0, 128]);
    let mask = rgba_fixture(2, 1, |x, _| {
        if x == 0 {
            [255, 255, 255, 0]
        } else {
            [255, 255, 255, 255]
        }
    });
    let (spec, map) = mask_recipe(
        0x43,
        &base,
        0x44,
        &mask,
        MaskChannel::Alpha,
        false,
        0.0,
        MaskMode::Replace,
    );
    let (request, transform) = limits();
    let output = png_output(2, 1, AlphaHandling::PreserveAlpha);
    let result = run_transform(&spec, &output, &map, &request, &transform).unwrap();
    let decoded = decode_output(&result.bytes);
    assert_eq!(decoded.get_pixel(0, 0).0, [255, 0, 0, 0]);
    assert_eq!(decoded.get_pixel(1, 0).0, [255, 0, 0, 255]);
}

#[test]
fn mask_invert_flips_coverage() {
    let base = rgba_fixture(2, 1, |_, _| [255, 0, 0, 255]);
    let mask = rgba_fixture(2, 1, |x, _| {
        if x == 0 {
            [255, 255, 255, 0]
        } else {
            [255, 255, 255, 255]
        }
    });
    let (spec, map) = mask_recipe(
        0x45,
        &base,
        0x46,
        &mask,
        MaskChannel::Alpha,
        true,
        0.0,
        MaskMode::Multiply,
    );
    let (request, transform) = limits();
    let output = png_output(2, 1, AlphaHandling::PreserveAlpha);
    let result = run_transform(&spec, &output, &map, &request, &transform).unwrap();
    let decoded = decode_output(&result.bytes);
    assert_eq!(decoded.get_pixel(0, 0).0[3], 255);
    assert_eq!(decoded.get_pixel(1, 0).0[3], 0);
}

#[test]
fn mask_luminance_channel_reads_brightness() {
    let base = rgba_fixture(1, 1, |_, _| [255, 0, 0, 255]);
    let mask = rgba_fixture(1, 1, |_, _| [255, 0, 0, 255]);
    let (spec, map) = mask_recipe(
        0x47,
        &base,
        0x48,
        &mask,
        MaskChannel::Luminance,
        false,
        0.0,
        MaskMode::Replace,
    );
    let (request, transform) = limits();
    let output = png_output(1, 1, AlphaHandling::PreserveAlpha);
    let result = run_transform(&spec, &output, &map, &request, &transform).unwrap();
    let decoded = decode_output(&result.bytes);
    assert_eq!(decoded.get_pixel(0, 0).0, [255, 0, 0, 54]);
}

#[test]
fn mask_dimension_mismatch_is_refused() {
    let base = rgba_fixture(2, 2, |_, _| [255, 0, 0, 255]);
    let mask = rgba_fixture(1, 1, |_, _| [255, 255, 255, 255]);
    let (spec, map) = mask_recipe(
        0x49,
        &base,
        0x4a,
        &mask,
        MaskChannel::Alpha,
        false,
        0.0,
        MaskMode::Replace,
    );
    let (request, transform) = limits();
    let output = png_output(2, 2, AlphaHandling::PreserveAlpha);
    assert_eq!(
        run_transform(&spec, &output, &map, &request, &transform).unwrap_err(),
        RasterError::DimensionMismatch {
            actual_width_px: 1,
            actual_height_px: 1,
            expected_width_px: 2,
            expected_height_px: 2,
        }
    );
}

#[test]
fn feather_softens_a_hard_coverage_edge() {
    let base = rgba_fixture(4, 1, |_, _| [255, 255, 255, 255]);
    let mask = rgba_fixture(4, 1, |x, _| {
        if x < 2 {
            [0, 0, 0, 255]
        } else {
            [255, 255, 255, 255]
        }
    });
    let (spec, map) = mask_recipe(
        0x4b,
        &base,
        0x4c,
        &mask,
        MaskChannel::Luminance,
        false,
        1.0,
        MaskMode::Replace,
    );
    let (request, transform) = limits();
    let output = png_output(4, 1, AlphaHandling::PreserveAlpha);
    let result = run_transform(&spec, &output, &map, &request, &transform).unwrap();
    let decoded = decode_output(&result.bytes);
    let left_edge = decoded.get_pixel(1, 0).0[3];
    let right_edge = decoded.get_pixel(2, 0).0[3];
    assert!((1..=254).contains(&left_edge), "left_edge={left_edge}");
    assert!((1..=254).contains(&right_edge), "right_edge={right_edge}");
}

fn composite_recipe(
    base: &RgbaImage,
    layers: Vec<(u8, RgbaImage, i32, i32, u16)>,
) -> (TransformSpec, HashMap<[u8; 16], Vec<u8>>) {
    let base_bytes = png_bytes(base);
    let mut map = HashMap::new();
    map.insert([0x51; 16], base_bytes.clone());
    let mut layer_refs = Vec::new();
    for (byte, image, x_px, y_px, opacity_permille) in &layers {
        let layer_bytes = png_bytes(image);
        map.insert([*byte; 16], layer_bytes.clone());
        layer_refs.push(
            CompositeLayer::new(
                pin(*byte, &layer_bytes),
                *x_px,
                *y_px,
                *opacity_permille,
                BlendMode::SourceOver,
            )
            .unwrap(),
        );
    }
    let spec = TransformSpec {
        source: TransformSource::Image(pin(0x51, &base_bytes)),
        steps: vec![ImageTransform::Composite(layer_refs)],
    };
    (spec, map)
}

#[test]
fn composite_layer_order_changes_the_result() {
    let base = rgba_fixture(1, 1, |_, _| [0, 0, 0, 255]);
    let red = rgba_fixture(1, 1, |_, _| [255, 0, 0, 255]);
    let blue = rgba_fixture(1, 1, |_, _| [0, 0, 255, 255]);
    let (request, transform) = limits();
    let output = png_output(1, 1, AlphaHandling::Opaque);
    let (red_first, red_first_map) = composite_recipe(
        &base,
        vec![(0x52, red.clone(), 0, 0, 1000), (0x53, blue.clone(), 0, 0, 1000)],
    );
    let (blue_first, blue_first_map) = composite_recipe(
        &base,
        vec![(0x53, blue.clone(), 0, 0, 1000), (0x52, red.clone(), 0, 0, 1000)],
    );
    let first = run_transform(&red_first, &output, &red_first_map, &request, &transform).unwrap();
    let second =
        run_transform(&blue_first, &output, &blue_first_map, &request, &transform).unwrap();
    assert_ne!(first.bytes, second.bytes);
    assert_eq!(decode_output(&first.bytes).get_pixel(0, 0).0, [0, 0, 255, 255]);
    assert_eq!(decode_output(&second.bytes).get_pixel(0, 0).0, [255, 0, 0, 255]);
}

#[test]
fn composite_zero_opacity_leaves_the_canvas_unchanged() {
    let base = rgba_fixture(1, 1, |_, _| [10, 20, 30, 255]);
    let red = rgba_fixture(1, 1, |_, _| [255, 0, 0, 255]);
    let (spec, map) = composite_recipe(&base, vec![(0x54, red, 0, 0, 0)]);
    let (request, transform) = limits();
    let output = png_output(1, 1, AlphaHandling::Opaque);
    let result = run_transform(&spec, &output, &map, &request, &transform).unwrap();
    assert_eq!(decode_output(&result.bytes).get_pixel(0, 0).0, [10, 20, 30, 255]);
}

#[test]
fn composite_half_opacity_blends_linearly() {
    let base = rgba_fixture(1, 1, |_, _| [0, 0, 0, 255]);
    let white = rgba_fixture(1, 1, |_, _| [255, 255, 255, 255]);
    let (spec, map) = composite_recipe(&base, vec![(0x55, white, 0, 0, 500)]);
    let (request, transform) = limits();
    let output = png_output(1, 1, AlphaHandling::Opaque);
    let result = run_transform(&spec, &output, &map, &request, &transform).unwrap();
    assert_eq!(decode_output(&result.bytes).get_pixel(0, 0).0, [128, 128, 128, 255]);
}

#[test]
fn composite_clips_layers_outside_bounds() {
    let base = rgba_fixture(2, 2, |_, _| [0, 0, 0, 255]);
    let red = rgba_fixture(2, 2, |_, _| [255, 0, 0, 255]);
    let (spec, map) = composite_recipe(&base, vec![(0x56, red, -1, -1, 1000)]);
    let (request, transform) = limits();
    let output = png_output(2, 2, AlphaHandling::Opaque);
    let result = run_transform(&spec, &output, &map, &request, &transform).unwrap();
    let decoded = decode_output(&result.bytes);
    assert_eq!(decoded.get_pixel(0, 0).0, [255, 0, 0, 255]);
    assert_eq!(decoded.get_pixel(1, 1).0, [0, 0, 0, 255]);
}

#[test]
fn matte_png_flattens_transparency_onto_the_fill() {
    let base = rgba_fixture(1, 1, |_, _| [255, 0, 0, 0]);
    let bytes = png_bytes(&base);
    let (spec, map) = recipe(0x61, &bytes, Vec::new());
    let (request, transform) = limits();
    let output = RasterOutputSpec::new(
        Dimensions::new(1, 1).unwrap(),
        AlphaHandling::Matte(Rgb {
            red: 0,
            green: 0,
            blue: 255,
        }),
        Encoding::Png,
    )
    .unwrap();
    let result = run_transform(&spec, &output, &map, &request, &transform).unwrap();
    assert!(!result.has_alpha);
    assert_eq!(result.mime, "image/png");
    assert_eq!(decode_output(&result.bytes).get_pixel(0, 0).0, [0, 0, 255, 255]);
}

#[test]
fn opaque_jpeg_from_opaque_pixels_needs_no_matte() {
    let rgb = RgbImage::from_pixel(2, 2, image::Rgb([200, 40, 40]));
    let bytes = jpeg_bytes(&rgb, 90);
    let (spec, map) = recipe(0x62, &bytes, Vec::new());
    let (request, transform) = limits();
    let output = RasterOutputSpec::new(
        Dimensions::new(2, 2).unwrap(),
        AlphaHandling::Opaque,
        Encoding::Jpeg {
            quality: LossyQuality::new(90).unwrap(),
        },
    )
    .unwrap();
    let result = run_transform(&spec, &output, &map, &request, &transform).unwrap();
    assert_eq!(result.mime, "image/jpeg");
    assert!(!result.has_alpha);
    let decoded = image::load_from_memory(&result.bytes).unwrap().to_rgb8();
    for pixel in decoded.pixels() {
        for (actual, expected) in pixel.0.iter().zip([200_u8, 40_u8, 40_u8]) {
            assert!((*actual as i16 - expected as i16).abs() <= 16);
        }
    }
}

#[test]
fn dropping_alpha_without_a_matte_is_refused() {
    let base = rgba_fixture(2, 2, |_, _| [255, 0, 0, 128]);
    let bytes = png_bytes(&base);
    let (spec, map) = recipe(0x63, &bytes, Vec::new());
    let (request, transform) = limits();
    let output = RasterOutputSpec::new(
        Dimensions::new(2, 2).unwrap(),
        AlphaHandling::Opaque,
        Encoding::Jpeg {
            quality: LossyQuality::new(90).unwrap(),
        },
    )
    .unwrap();
    assert_eq!(
        run_transform(&spec, &output, &map, &request, &transform).unwrap_err(),
        RasterError::InvalidOutput(crate::output::OutputError::AlphaWithoutMatte)
    );
}

#[test]
fn lossy_webp_output_is_explicitly_unsupported() {
    let base = rgba_fixture(2, 2, |_, _| [9, 9, 9, 255]);
    let bytes = png_bytes(&base);
    let (spec, map) = recipe(0x64, &bytes, Vec::new());
    let (request, transform) = limits();
    let output = RasterOutputSpec::new(
        Dimensions::new(2, 2).unwrap(),
        AlphaHandling::PreserveAlpha,
        Encoding::WebP {
            quality: Some(LossyQuality::new(80).unwrap()),
        },
    )
    .unwrap();
    assert_eq!(
        run_transform(&spec, &output, &map, &request, &transform).unwrap_err(),
        RasterError::UnsupportedFormat
    );
}

#[test]
fn lossless_webp_round_trips_alpha() {
    let base = rgba_fixture(2, 1, |x, _| {
        if x == 0 {
            [255, 0, 0, 0]
        } else {
            [0, 255, 0, 255]
        }
    });
    let bytes = png_bytes(&base);
    let (spec, map) = recipe(0x65, &bytes, Vec::new());
    let (request, transform) = limits();
    let output = RasterOutputSpec::new(
        Dimensions::new(2, 1).unwrap(),
        AlphaHandling::PreserveAlpha,
        Encoding::WebP { quality: None },
    )
    .unwrap();
    let result = run_transform(&spec, &output, &map, &request, &transform).unwrap();
    assert_eq!(result.mime, "image/webp");
    assert!(result.has_alpha);
    let decoded = decode_output(&result.bytes);
    assert_eq!(decoded.get_pixel(0, 0).0, [255, 0, 0, 0]);
    assert_eq!(decoded.get_pixel(1, 0).0, [0, 255, 0, 255]);
}

#[test]
fn encoded_output_over_limit_is_refused() {
    let base = rgba_fixture(2, 2, |_, _| [9, 9, 9, 255]);
    let bytes = png_bytes(&base);
    let (spec, map) = recipe(0x66, &bytes, Vec::new());
    let (request, mut transform) = limits();
    transform.max_output_bytes = 10;
    let output = png_output(2, 2, AlphaHandling::Opaque);
    assert!(matches!(
        run_transform(&spec, &output, &map, &request, &transform).unwrap_err(),
        RasterError::OutputTooLarge { .. }
    ));
}

#[test]
fn final_dimensions_must_match_the_output() {
    let base = rgba_fixture(2, 2, |_, _| [9, 9, 9, 255]);
    let bytes = png_bytes(&base);
    let (spec, map) = recipe(0x67, &bytes, Vec::new());
    let (request, transform) = limits();
    let output = png_output(4, 4, AlphaHandling::Opaque);
    assert_eq!(
        run_transform(&spec, &output, &map, &request, &transform).unwrap_err(),
        RasterError::DimensionMismatch {
            actual_width_px: 2,
            actual_height_px: 2,
            expected_width_px: 4,
            expected_height_px: 4,
        }
    );
}

#[test]
fn exif_orientation_six_parses_from_little_endian_app1() {
    let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xE1, 0x00, 0x22];
    bytes.extend_from_slice(b"Exif\0\0");
    bytes.extend_from_slice(&[
        0x49, 0x49, 0x2A, 0x00, 0x08, 0x00, 0x00, 0x00, 0x01, 0x00, 0x12, 0x01, 0x03, 0x00,
        0x01, 0x00, 0x00, 0x00, 0x06, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ]);
    assert_eq!(crate::raster::decode::parse_exif_orientation(&bytes), 6);
}

#[test]
fn orientation_eight_swaps_dimensions() {
    let row = rgba_fixture(2, 1, |x, _| {
        if x == 0 {
            [255, 0, 0, 255]
        } else {
            [0, 0, 255, 255]
        }
    });
    let rotated = crate::raster::decode::apply_orientation(row, 8);
    assert_eq!((rotated.width(), rotated.height()), (1, 2));
    assert_eq!(rotated.get_pixel(0, 0).0, [0, 0, 255, 255]);
    assert_eq!(rotated.get_pixel(0, 1).0, [255, 0, 0, 255]);
}

#[test]
fn corrupt_exif_falls_back_to_identity() {
    assert_eq!(crate::raster::decode::parse_exif_orientation(b"not a jpeg"), 1);
    let row = rgba_fixture(2, 1, |_, _| [9, 9, 9, 255]);
    let kept = crate::raster::decode::apply_orientation(row.clone(), 9);
    assert_eq!((kept.width(), kept.height()), (2, 1));
}

#[test]
fn coverage_source_decodes_like_an_image_source() {
    let gray = rgba_fixture(2, 1, |x, _| {
        if x == 0 {
            [0, 0, 0, 255]
        } else {
            [255, 255, 255, 255]
        }
    });
    let bytes = png_bytes(&gray);
    let coverage = CoverageRef {
        coverage: pin(0x71, &bytes),
        analyzed: pin(0x72, &bytes),
        analysis_revision: 3,
    };
    let mut map = HashMap::new();
    map.insert([0x71; 16], bytes.clone());
    map.insert([0x72; 16], bytes);
    let spec = TransformSpec {
        source: TransformSource::Coverage(coverage),
        steps: Vec::new(),
    };
    let (request, transform) = limits();
    let output = png_output(2, 1, AlphaHandling::Opaque);
    let result = run_transform(&spec, &output, &map, &request, &transform).unwrap();
    let decoded = decode_output(&result.bytes);
    assert_eq!(decoded.get_pixel(0, 0).0, [0, 0, 0, 255]);
    assert_eq!(decoded.get_pixel(1, 0).0, [255, 255, 255, 255]);
}

#[test]
fn box_blur_peak_is_reserved_before_allocation() {
    let base = rgba_fixture(8, 8, |_, _| [200, 40, 40, 255]);
    let bytes = png_bytes(&base);
    let steps = vec![ImageTransform::Blur(
        BlurSpec::new(2.0, BlurAlgorithm::Box).unwrap(),
    )];
    let (spec, map) = recipe(0x81, &bytes, steps);
    let (request, mut transform) = limits();
    transform.max_decoded_bytes = 8 * 8 * 4 + 8 * 8 * 24 - 1;
    let output = png_output(8, 8, AlphaHandling::Opaque);
    assert!(matches!(
        run_transform(&spec, &output, &map, &request, &transform).unwrap_err(),
        RasterError::LiveBudgetExceeded { .. }
    ));
}

#[test]
fn box_blur_transparent_edge_keeps_no_halo() {
    let base = rgba_fixture(3, 1, |x, _| match x {
        0 => [0, 0, 0, 0],
        1 => [255, 0, 0, 255],
        _ => [0, 0, 0, 0],
    });
    let bytes = png_bytes(&base);
    let steps = vec![ImageTransform::Blur(
        BlurSpec::new(1.0, BlurAlgorithm::Box).unwrap(),
    )];
    let (spec, map) = recipe(0x82, &bytes, steps);
    let (request, transform) = limits();
    let output = png_output(3, 1, AlphaHandling::PreserveAlpha);
    let result = run_transform(&spec, &output, &map, &request, &transform).unwrap();
    let decoded = decode_output(&result.bytes);
    assert_eq!(decoded.get_pixel(1, 0).0[1], 0);
    assert_eq!(decoded.get_pixel(1, 0).0[2], 0);
    let edge = decoded.get_pixel(0, 0).0;
    assert_eq!(edge[0], 255, "premultiplied red edge={edge:?}");
    assert!(edge[1] == 0 && edge[2] == 0, "edge={edge:?}");
}

#[test]
fn composite_half_opacity_mixes_in_linear_light() {
    let base = rgba_fixture(1, 1, |_, _| [0, 0, 0, 255]);
    let white = rgba_fixture(1, 1, |_, _| [255, 255, 255, 255]);
    let (spec, map) = composite_recipe(&base, vec![(0x83, white, 0, 0, 500)]);
    let (request, transform) = limits();
    let output = png_output(1, 1, AlphaHandling::Opaque);
    let result = run_transform(&spec, &output, &map, &request, &transform).unwrap();
    let decoded = decode_output(&result.bytes).get_pixel(0, 0).0;
    assert!(
        (180..=195).contains(&decoded[0]),
        "linear mid grey={decoded:?}"
    );
}

#[test]
fn matte_flatten_mixes_in_linear_light() {
    let base = rgba_fixture(1, 1, |_, _| [255, 0, 0, 128]);
    let bytes = png_bytes(&base);
    let (spec, map) = recipe(0x84, &bytes, Vec::new());
    let (request, transform) = limits();
    let output = RasterOutputSpec::new(
        Dimensions::new(1, 1).unwrap(),
        AlphaHandling::Matte(Rgb {
            red: 0,
            green: 0,
            blue: 0,
        }),
        Encoding::Png,
    )
    .unwrap();
    let result = run_transform(&spec, &output, &map, &request, &transform).unwrap();
    let decoded = decode_output(&result.bytes).get_pixel(0, 0).0;
    assert!(
        (180..=195).contains(&decoded[0]),
        "linear red on black={decoded:?}"
    );
    assert_eq!(decoded[1], 0);
    assert_eq!(decoded[2], 0);
}

#[test]
fn luminance_mask_keeps_luma_for_transparent_red() {
    let base = rgba_fixture(1, 1, |_, _| [255, 0, 0, 255]);
    let mask = rgba_fixture(1, 1, |_, _| [255, 0, 0, 0]);
    let (spec, map) = mask_recipe(
        0x85,
        &base,
        0x86,
        &mask,
        MaskChannel::Luminance,
        false,
        0.0,
        MaskMode::Replace,
    );
    let (request, transform) = limits();
    let output = png_output(1, 1, AlphaHandling::PreserveAlpha);
    let result = run_transform(&spec, &output, &map, &request, &transform).unwrap();
    let decoded = decode_output(&result.bytes);
    assert_eq!(decoded.get_pixel(0, 0).0, [255, 0, 0, 54]);
}

#[test]
fn png_with_iccp_chunk_is_refused_not_silently_dropped() {
    let base = rgba_fixture(2, 2, |_, _| [9, 9, 9, 255]);
    let mut bytes = png_bytes(&base);
    let iccp = make_png_iccp_chunk();
    let iend = bytes.len() - 12;
    bytes.splice(iend..iend, iccp);
    let (spec, map) = recipe(0x87, &bytes, Vec::new());
    let (request, transform) = limits();
    let output = png_output(2, 2, AlphaHandling::Opaque);
    assert_eq!(
        run_transform(&spec, &output, &map, &request, &transform).unwrap_err(),
        RasterError::UnsupportedColorProfile
    );
}

fn make_png_iccp_chunk() -> Vec<u8> {
    let mut chunk = Vec::new();
    chunk.extend_from_slice(&7_u32.to_be_bytes());
    chunk.extend_from_slice(b"iCCP");
    chunk.extend_from_slice(b"profile\0\0\x00fakematter");
    chunk.extend_from_slice(&0_u32.to_be_bytes());
    chunk
}

#[test]
fn arbitrary_ninety_with_fixed_canvas_keeps_dimensions() {
    let row = rgba_fixture(2, 1, |x, _| {
        if x == 0 {
            [255, 0, 0, 255]
        } else {
            [0, 0, 255, 255]
        }
    });
    let bytes = png_bytes(&row);
    let arbitrary = ImageTransform::Rotate(RotateSpec::Arbitrary {
        degrees: 90.0,
        expand_canvas: false,
        fill: Rgb {
            red: 0,
            green: 0,
            blue: 0,
        },
        interpolation: Interpolation::Bilinear,
    });
    let (spec, map) = recipe(0x88, &bytes, vec![arbitrary]);
    let (request, transform) = limits();
    let output = png_output(2, 1, AlphaHandling::Opaque);
    let result = run_transform(&spec, &output, &map, &request, &transform).unwrap();
    assert_eq!((result.width_px, result.height_px), (2, 1));
}

#[test]
fn cancellation_stops_brightness_on_large_raster() {
    let base = rgba_fixture(64, 64, |_, _| [9, 9, 9, 255]);
    let bytes = png_bytes(&base);
    let steps = vec![ImageTransform::Brighten(
        BrightnessOffset::new(0.5).unwrap(),
    )];
    let source = pin(0x89, &bytes);
    let spec = TransformSpec {
        source: TransformSource::Image(source),
        steps,
    };
    let mut map = HashMap::new();
    map.insert([0x89; 16], bytes);
    let (request, transform) = limits();
    let output = png_output(64, 64, AlphaHandling::Opaque);
    let calls = std::cell::Cell::new(0_u32);
    let check = || -> Result<(), RasterError> {
        calls.set(calls.get() + 1);
        if calls.get() > 2 {
            return Err(RasterError::Cancelled);
        }
        Ok(())
    };
    let outcome = execute_transform(
        &spec,
        &output,
        &request,
        &transform,
        &resolver_for(&map),
        &check,
    );
    assert_eq!(outcome.unwrap_err(), RasterError::Cancelled);
}

#[test]
fn encoded_output_stops_at_the_output_limit() {
    let base = rgba_fixture(64, 64, |x, y| {
        if (x + y) % 2 == 0 {
            [200, 40, 40, 255]
        } else {
            [40, 200, 40, 255]
        }
    });
    let bytes = png_bytes(&base);
    let (spec, map) = recipe(0x8a, &bytes, Vec::new());
    let (request, mut transform) = limits();
    transform.max_output_bytes = 64;
    let output = png_output(64, 64, AlphaHandling::Opaque);
    assert!(matches!(
        run_transform(&spec, &output, &map, &request, &transform).unwrap_err(),
        RasterError::OutputTooLarge { .. }
    ));
}
