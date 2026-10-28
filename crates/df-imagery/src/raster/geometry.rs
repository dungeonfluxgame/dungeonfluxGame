//! Ordered geometry over the current oriented raster.
//!
//! Coordinates always describe the current raster: crop-then-resize and
//! resize-then-crop are different requests by construction. `Contain` keeps
//! the scaled content size, `Cover` and `Stretch` fill the exact bounds, and
//! `Pad` always returns the exact canvas on its explicit fill. Resampling
//! integrates in linear light with premultiplied alpha so transparent edges
//! do not fringe; encoded-space averaging would halo.

use image::{GenericImageView, Rgba, RgbaImage, imageops::FilterType};

use super::CancelCheck;
use super::RasterError;
use super::bounds::{self, LiveBudget};
use super::color::{linear_to_srgb_byte, srgb_byte_to_linear};
use crate::transform::{Anchor1D, Interpolation, PadSpec, PixelRect, ResizeMode, ResizeSpec};

/// Maps the frozen interpolation vocabulary onto the pinned resampler.
pub(crate) fn filter_for(interpolation: Interpolation) -> FilterType {
    match interpolation {
        Interpolation::Nearest => FilterType::Nearest,
        Interpolation::Bilinear => FilterType::Triangle,
        Interpolation::Bicubic => FilterType::CatmullRom,
        Interpolation::Lanczos3 => FilterType::Lanczos3,
    }
}

/// Scales the content per the resize mode; modes that would enlarge without
/// `allow_upscale` leave the raster unchanged and let the final dimension
/// check report the mismatch. Resampling is linear-light premultiplied.
pub(crate) fn apply_resize(
    image: RgbaImage,
    spec: &ResizeSpec,
    budget: &LiveBudget,
    check_cancel: CancelCheck<'_>,
) -> Result<RgbaImage, RasterError> {
    let (width_px, height_px) = image.dimensions();
    let target_width_px = spec.width_px();
    let target_height_px = spec.height_px();
    match spec.mode() {
        ResizeMode::Contain => {
            let (scaled_width_px, scaled_height_px) =
                contain_size(width_px, height_px, target_width_px, target_height_px);
            if !spec.allow_upscale()
                && (scaled_width_px > width_px || scaled_height_px > height_px)
            {
                return Ok(image);
            }
            resample_linear_checked(
                &image,
                scaled_width_px,
                scaled_height_px,
                spec,
                budget,
                check_cancel,
            )
        }
        ResizeMode::Cover => {
            let (scaled_width_px, scaled_height_px) =
                cover_size(width_px, height_px, target_width_px, target_height_px);
            if !spec.allow_upscale()
                && (scaled_width_px > width_px || scaled_height_px > height_px)
            {
                return Ok(image);
            }
            let scaled = resample_linear_checked(
                &image,
                scaled_width_px,
                scaled_height_px,
                spec,
                budget,
                check_cancel,
            )?;
            let offset_x_px =
                cover_offset(scaled_width_px, target_width_px, spec.anchor().x);
            let offset_y_px =
                cover_offset(scaled_height_px, target_height_px, spec.anchor().y);
            crop_pixels(
                &scaled,
                offset_x_px,
                offset_y_px,
                target_width_px,
                target_height_px,
                budget,
                check_cancel,
            )
        }
        ResizeMode::Stretch => {
            if !spec.allow_upscale()
                && (target_width_px > width_px || target_height_px > height_px)
            {
                return Ok(image);
            }
            resample_linear_checked(
                &image,
                target_width_px,
                target_height_px,
                spec,
                budget,
                check_cancel,
            )
        }
        ResizeMode::Pad => {
            let (mut content_width_px, mut content_height_px) =
                contain_size(width_px, height_px, target_width_px, target_height_px);
            if !spec.allow_upscale() {
                content_width_px = content_width_px.min(width_px);
                content_height_px = content_height_px.min(height_px);
            }
            // `ResizeSpec::new` refuses `Pad` without a fill, so this is a
            // proven construction invariant, never a caller default.
            let fill = spec.pad_fill().expect("pad fill is enforced by ResizeSpec::new");
            budget.check_pixels(target_width_px, target_height_px)?;
            budget.check_retained(
                bounds::rgba_bytes(width_px, height_px),
                bounds::rgba_bytes(target_width_px, target_height_px),
            )?;
            check_cancel()?;
            let content = if content_width_px == width_px && content_height_px == height_px {
                image
            } else {
                resample_linear_checked(
                    &image,
                    content_width_px,
                    content_height_px,
                    spec,
                    budget,
                    check_cancel,
                )?
            };
            let mut canvas = RgbaImage::from_pixel(
                target_width_px,
                target_height_px,
                Rgba([fill.red, fill.green, fill.blue, 255]),
            );
            blit_onto(
                &mut canvas,
                &content,
                bounds::anchor_start(target_width_px, content_width_px, spec.anchor().x),
                bounds::anchor_start(target_height_px, content_height_px, spec.anchor().y),
                check_cancel,
            )?;
            Ok(canvas)
        }
    }
}

/// Copies the rectangle; out-of-bounds rectangles are refused because the
/// rectangle carries no padding request.
pub(crate) fn apply_crop(
    image: RgbaImage,
    rect: &PixelRect,
    budget: &LiveBudget,
    check_cancel: CancelCheck<'_>,
) -> Result<RgbaImage, RasterError> {
    check_cancel()?;
    crop_pixels(
        &image,
        rect.x_px(),
        rect.y_px(),
        rect.width_px(),
        rect.height_px(),
        budget,
        check_cancel,
    )
}

fn crop_pixels(
    image: &RgbaImage,
    x_px: u32,
    y_px: u32,
    width_px: u32,
    height_px: u32,
    budget: &LiveBudget,
    check_cancel: CancelCheck<'_>,
) -> Result<RgbaImage, RasterError> {
    let (image_width_px, image_height_px) = image.dimensions();
    let end_x_px = x_px.checked_add(width_px).unwrap_or(u32::MAX);
    let end_y_px = y_px.checked_add(height_px).unwrap_or(u32::MAX);
    if end_x_px > image_width_px || end_y_px > image_height_px {
        return Err(RasterError::CropOutOfBounds);
    }
    budget.check_retained(
        bounds::rgba_bytes(image_width_px, image_height_px),
        bounds::rgba_bytes(width_px, height_px),
    )?;
    let mut cropped = RgbaImage::new(width_px, height_px);
    for row in 0..height_px {
        if row % 16 == 0 {
            check_cancel()?;
        }
        for col in 0..width_px {
            cropped.put_pixel(col, row, *image.get_pixel(x_px + col, y_px + row));
        }
    }
    Ok(cropped)
}

/// Places the content on the exact canvas; overhanging content is clipped at
/// the anchor rather than refused.
pub(crate) fn apply_pad(
    image: RgbaImage,
    spec: &PadSpec,
    budget: &LiveBudget,
    check_cancel: CancelCheck<'_>,
) -> Result<RgbaImage, RasterError> {
    let (width_px, height_px) = image.dimensions();
    let target_width_px = spec.width_px();
    let target_height_px = spec.height_px();
    budget.check_pixels(target_width_px, target_height_px)?;
    budget.check_retained(
        bounds::rgba_bytes(width_px, height_px),
        bounds::rgba_bytes(target_width_px, target_height_px),
    )?;
    check_cancel()?;
    let fill = spec.fill();
    let mut canvas = RgbaImage::from_pixel(
        target_width_px,
        target_height_px,
        Rgba([fill.red, fill.green, fill.blue, 255]),
    );
    blit_onto(
        &mut canvas,
        &image,
        bounds::anchor_start(target_width_px, width_px, spec.anchor().x),
        bounds::anchor_start(target_height_px, height_px, spec.anchor().y),
        check_cancel,
    )?;
    Ok(canvas)
}

fn blit_onto(
    canvas: &mut RgbaImage,
    content: &RgbaImage,
    start_x_px: i64,
    start_y_px: i64,
    check_cancel: CancelCheck<'_>,
) -> Result<(), RasterError> {
    let (canvas_width_px, canvas_height_px) = canvas.dimensions();
    let (content_width_px, content_height_px) = content.dimensions();
    for row in 0..content_height_px {
        if row % 16 == 0 {
            check_cancel()?;
        }
        let target_y_px = start_y_px + i64::from(row);
        if target_y_px < 0 || target_y_px >= i64::from(canvas_height_px) {
            continue;
        }
        for col in 0..content_width_px {
            let target_x_px = start_x_px + i64::from(col);
            if target_x_px < 0 || target_x_px >= i64::from(canvas_width_px) {
                continue;
            }
            canvas.put_pixel(
                target_x_px as u32,
                target_y_px as u32,
                *content.get_pixel(col, row),
            );
        }
    }
    Ok(())
}

/// Exact quarter turns first; arbitrary angles resample with the named
/// interpolation on the explicit canvas and fill color. Exact multiples of
/// a right angle named as arbitrary honor the fixed canvas: with
/// `expand_canvas` false the output keeps the input dimensions and the
/// rotated content is clipped onto the fill, only an expanded canvas swaps
/// dimensions through the exact path.
pub(crate) fn apply_rotate(
    image: RgbaImage,
    spec: &crate::transform::RotateSpec,
    budget: &LiveBudget,
    check_cancel: CancelCheck<'_>,
) -> Result<RgbaImage, RasterError> {
    match *spec {
        crate::transform::RotateSpec::QuarterTurns(turns) => {
            let (width_px, height_px) = image.dimensions();
            let swapped = turns % 4 == 1 || turns % 4 == 3;
            let (out_width_px, out_height_px) = if swapped {
                (height_px, width_px)
            } else {
                (width_px, height_px)
            };
            budget.check_retained(
                bounds::rgba_bytes(width_px, height_px),
                bounds::rgba_bytes(out_width_px, out_height_px),
            )?;
            check_cancel()?;
            Ok(quarter_turn(&image, turns))
        }
        crate::transform::RotateSpec::Arbitrary {
            degrees,
            expand_canvas,
            fill,
            interpolation,
        } => {
            let normalized_degrees = degrees.rem_euclid(360.0);
            if normalized_degrees == 0.0 {
                return Ok(image);
            }
            if expand_canvas
                && (normalized_degrees == 90.0
                    || normalized_degrees == 180.0
                    || normalized_degrees == 270.0)
            {
                let (width_px, height_px) = image.dimensions();
                let swapped =
                    normalized_degrees == 90.0 || normalized_degrees == 270.0;
                let (out_width_px, out_height_px) = if swapped {
                    (height_px, width_px)
                } else {
                    (width_px, height_px)
                };
                budget.check_retained(
                    bounds::rgba_bytes(width_px, height_px),
                    bounds::rgba_bytes(out_width_px, out_height_px),
                )?;
                check_cancel()?;
                if normalized_degrees == 90.0 {
                    return Ok(rotate_90_clockwise(&image));
                }
                if normalized_degrees == 180.0 {
                    return Ok(rotate_180(&image));
                }
                return Ok(rotate_270_counter_clockwise(&image));
            }
            rotate_arbitrary(
                &image,
                normalized_degrees,
                expand_canvas,
                fill,
                interpolation,
                budget,
                check_cancel,
            )
        }
    }
}

/// Mirrors the raster on one axis; exact, no resampling involved.
pub(crate) fn apply_flip(
    image: RgbaImage,
    axis: crate::transform::FlipAxis,
    budget: &LiveBudget,
    check_cancel: CancelCheck<'_>,
) -> Result<RgbaImage, RasterError> {
    let (width_px, height_px) = image.dimensions();
    budget.check_retained(
        bounds::rgba_bytes(width_px, height_px),
        bounds::rgba_bytes(width_px, height_px),
    )?;
    check_cancel()?;
    let flipped = match axis {
        crate::transform::FlipAxis::Horizontal => flip_horizontal(&image),
        crate::transform::FlipAxis::Vertical => flip_vertical(&image),
    };
    check_cancel()?;
    Ok(flipped)
}

fn quarter_turn(image: &RgbaImage, turns: u8) -> RgbaImage {
    match turns % 4 {
        0 => image.clone(),
        1 => rotate_90_clockwise(image),
        2 => rotate_180(image),
        _ => rotate_270_counter_clockwise(image),
    }
}

/// Clockwise right angle: output pixel `(x, y)` samples input `(y, h - 1 - x)`.
pub(crate) fn rotate_90_clockwise(image: &RgbaImage) -> RgbaImage {
    let (width_px, height_px) = image.dimensions();
    let mut out = RgbaImage::new(height_px, width_px);
    for y in 0..width_px {
        for x in 0..height_px {
            out.put_pixel(x, y, *image.get_pixel(y, height_px - 1 - x));
        }
    }
    out
}

pub(crate) fn rotate_180(image: &RgbaImage) -> RgbaImage {
    let (width_px, height_px) = image.dimensions();
    let mut out = RgbaImage::new(width_px, height_px);
    for y in 0..height_px {
        for x in 0..width_px {
            out.put_pixel(x, y, *image.get_pixel(width_px - 1 - x, height_px - 1 - y));
        }
    }
    out
}

/// Counter-clockwise right angle: output `(x, y)` samples `(w - 1 - y, x)`.
pub(crate) fn rotate_270_counter_clockwise(image: &RgbaImage) -> RgbaImage {
    let (width_px, height_px) = image.dimensions();
    let mut out = RgbaImage::new(height_px, width_px);
    for y in 0..width_px {
        for x in 0..height_px {
            out.put_pixel(x, y, *image.get_pixel(width_px - 1 - y, x));
        }
    }
    out
}

pub(crate) fn flip_horizontal(image: &RgbaImage) -> RgbaImage {
    let (width_px, height_px) = image.dimensions();
    let mut out = RgbaImage::new(width_px, height_px);
    for y in 0..height_px {
        for x in 0..width_px {
            out.put_pixel(x, y, *image.get_pixel(width_px - 1 - x, y));
        }
    }
    out
}

pub(crate) fn flip_vertical(image: &RgbaImage) -> RgbaImage {
    let (width_px, height_px) = image.dimensions();
    let mut out = RgbaImage::new(width_px, height_px);
    for y in 0..height_px {
        for x in 0..width_px {
            out.put_pixel(x, y, *image.get_pixel(x, height_px - 1 - y));
        }
    }
    out
}

fn contain_size(
    width_px: u32,
    height_px: u32,
    bound_width_px: u32,
    bound_height_px: u32,
) -> (u32, u32) {
    // Width-constrained when the target is relatively narrower.
    let width_constrained = u64::from(bound_width_px).saturating_mul(u64::from(height_px))
        < u64::from(bound_height_px).saturating_mul(u64::from(width_px));
    if width_constrained {
        (
            bound_width_px,
            scale_dimension(height_px, bound_width_px, width_px),
        )
    } else {
        (
            scale_dimension(width_px, bound_height_px, height_px),
            bound_height_px,
        )
    }
}

fn cover_size(
    width_px: u32,
    height_px: u32,
    bound_width_px: u32,
    bound_height_px: u32,
) -> (u32, u32) {
    // Cover is the dual of contain: fill the bound, then crop the overhang.
    let width_constrained = u64::from(bound_width_px).saturating_mul(u64::from(height_px))
        >= u64::from(bound_height_px).saturating_mul(u64::from(width_px));
    if width_constrained {
        (
            bound_width_px,
            scale_dimension(height_px, bound_width_px, width_px),
        )
    } else {
        (
            scale_dimension(width_px, bound_height_px, height_px),
            bound_height_px,
        )
    }
}

/// Crop window origin inside the cover-scaled raster; `cover_size` fills the
/// bound up to rounding, so the saturating subtraction only guards the final
/// unit rather than deciding layout.
fn cover_offset(scaled_len_px: u32, bound_len_px: u32, axis: Anchor1D) -> u32 {
    let overhang_px = scaled_len_px.saturating_sub(bound_len_px);
    match axis {
        Anchor1D::Start => 0,
        Anchor1D::Center => overhang_px / 2,
        Anchor1D::End => overhang_px,
    }
}

/// Nearest-integer rescale of one axis; zero can only come from a zero
/// source axis, which constructors already refuse, but the floor keeps the
/// size representable either way.
fn scale_dimension(value_px: u32, numerator: u32, denominator: u32) -> u32 {
    if denominator == 0 {
        return 1;
    }
    let scaled = u64::from(value_px).saturating_mul(u64::from(numerator));
    let rounded = scaled
        .checked_add(u64::from(denominator) / 2)
        .unwrap_or(u64::MAX)
        / u64::from(denominator);
    rounded.min(u64::from(u32::MAX)) as u32
}

fn resample_linear_checked(
    image: &RgbaImage,
    width_px: u32,
    height_px: u32,
    spec: &ResizeSpec,
    budget: &LiveBudget,
    check_cancel: CancelCheck<'_>,
) -> Result<RgbaImage, RasterError> {
    budget.check_pixels(width_px, height_px)?;
    budget.check_retained(
        bounds::rgba_bytes(image.width(), image.height()),
        bounds::rgba_bytes(width_px, height_px),
    )?;
    check_cancel()?;
    // The pinned library resampler is intentionally not used here: it
    // averages encoded bytes with straight alpha. The loop below integrates
    // in linear light with premultiplied alpha and yields to cancellation.
    let filter = filter_for(spec.interpolation());
    let (source_width_px, source_height_px) = image.dimensions();
    let mut out = RgbaImage::new(width_px, height_px);
    if width_px == 0 || height_px == 0 {
        return Err(RasterError::DecodedTooLarge { pixels: 0 });
    }
    for row in 0..height_px {
        if row % 16 == 0 {
            check_cancel()?;
        }
        let source_y =
            (bounds::dim_to_f32(row) + 0.5) * bounds::dim_to_f32(source_height_px) / bounds::dim_to_f32(height_px) - 0.5;
        for col in 0..width_px {
            let source_x = (bounds::dim_to_f32(col) + 0.5) * bounds::dim_to_f32(source_width_px)
                / bounds::dim_to_f32(width_px)
                - 0.5;
            out.put_pixel(col, row, sample_source(image, source_x, source_y, filter));
        }
    }
    check_cancel()?;
    Ok(out)
}

#[allow(clippy::too_many_arguments, reason = "rotation carries its full canvas policy")]
fn rotate_arbitrary(
    image: &RgbaImage,
    degrees: f32,
    expand_canvas: bool,
    fill: crate::output::Rgb,
    interpolation: Interpolation,
    budget: &LiveBudget,
    check_cancel: CancelCheck<'_>,
) -> Result<RgbaImage, RasterError> {
    let (width_px, height_px) = image.dimensions();
    let radians = degrees.to_radians();
    let (cosine, sine) = (radians.cos(), radians.sin());
    let (canvas_width_px, canvas_height_px) = if expand_canvas {
        let corners = [
            rotate_point(0.0, 0.0, cosine, sine),
            rotate_point(bounds::dim_to_f32(width_px), 0.0, cosine, sine),
            rotate_point(0.0, bounds::dim_to_f32(height_px), cosine, sine),
            rotate_point(
                bounds::dim_to_f32(width_px),
                bounds::dim_to_f32(height_px),
                cosine,
                sine,
            ),
        ];
        let min_x = corners.iter().map(|corner| corner.0).fold(f32::INFINITY, f32::min);
        let max_x = corners.iter().map(|corner| corner.0).fold(f32::NEG_INFINITY, f32::max);
        let min_y = corners.iter().map(|corner| corner.1).fold(f32::INFINITY, f32::min);
        let max_y = corners.iter().map(|corner| corner.1).fold(f32::NEG_INFINITY, f32::max);
        ((max_x - min_x).ceil().max(1.0) as u32, (max_y - min_y).ceil().max(1.0) as u32)
    } else {
        (width_px, height_px)
    };
    budget.check_pixels(canvas_width_px, canvas_height_px)?;
    budget.check_retained(
        bounds::rgba_bytes(width_px, height_px),
        bounds::rgba_bytes(canvas_width_px, canvas_height_px),
    )?;
    let source_center_x = bounds::dim_to_f32(width_px) / 2.0;
    let source_center_y = bounds::dim_to_f32(height_px) / 2.0;
    let canvas_center_x = bounds::dim_to_f32(canvas_width_px) / 2.0;
    let canvas_center_y = bounds::dim_to_f32(canvas_height_px) / 2.0;
    let fill = Rgba([fill.red, fill.green, fill.blue, 255]);
    let mut canvas = RgbaImage::from_pixel(canvas_width_px, canvas_height_px, fill);
    let filter = filter_for(interpolation);
    for row in 0..canvas_height_px {
        if row % 16 == 0 {
            check_cancel()?;
        }
        for col in 0..canvas_width_px {
            let centered_x = bounds::dim_to_f32(col) + 0.5 - canvas_center_x;
            let centered_y = bounds::dim_to_f32(row) + 0.5 - canvas_center_y;
            // Inverse rotation maps the output pixel to its source sample.
            let source_x = cosine * centered_x + sine * centered_y + source_center_x - 0.5;
            let source_y = -sine * centered_x + cosine * centered_y + source_center_y - 0.5;
            if source_x >= 0.0
                && source_y >= 0.0
                && source_x < bounds::dim_to_f32(width_px)
                && source_y < bounds::dim_to_f32(height_px)
            {
                canvas.put_pixel(col, row, sample_source(image, source_x, source_y, filter));
            }
        }
    }
    Ok(canvas)
}

fn rotate_point(x: f32, y: f32, cosine: f32, sine: f32) -> (f32, f32) {
    (cosine * x - sine * y, sine * x + cosine * y)
}

/// Samples the source with the named kernel in linear light with
/// premultiplied alpha so transparent edges do not fringe, and fully
/// transparent samples contribute nothing.
fn sample_source(image: &RgbaImage, x: f32, y: f32, filter: FilterType) -> Rgba<u8> {
    let (width_px, height_px) = image.dimensions();
    let clamp_index = |value: i64, bound: u32| -> u32 {
        value.clamp(0, i64::from(bound) - 1) as u32
    };
    if filter == FilterType::Nearest {
        let pixel = image.get_pixel(
            clamp_index(x.round() as i64, width_px),
            clamp_index(y.round() as i64, height_px),
        );
        return *pixel;
    }
    let radius = match filter {
        FilterType::Triangle => 1.0,
        FilterType::CatmullRom => 2.0,
        _ => 3.0,
    };
    let left = (x - radius).floor() as i64;
    let top = (y - radius).floor() as i64;
    let right = (x + radius).ceil() as i64;
    let bottom = (y + radius).ceil() as i64;
    let mut sum_weights = 0.0_f32;
    let mut sum_premultiplied = [0.0_f32; 3];
    let mut sum_alpha = 0.0_f32;
    for sample_y in top..=bottom {
        let weight_y = kernel_weight(y - sample_y as f32, filter);
        for sample_x in left..=right {
            let weight = weight_y * kernel_weight(x - sample_x as f32, filter);
            if weight == 0.0 {
                continue;
            }
            let pixel = image.get_pixel(
                clamp_index(sample_x, width_px),
                clamp_index(sample_y, height_px),
            );
            let alpha = f32::from(pixel[3]) / 255.0;
            sum_weights += weight;
            sum_alpha += weight * alpha;
            for channel in 0..3 {
                sum_premultiplied[channel] += weight * srgb_byte_to_linear(pixel[channel]) * alpha;
            }
        }
    }
    if sum_weights <= 0.0 || sum_alpha <= 0.0 {
        return Rgba([0, 0, 0, 0]);
    }
    let alpha = (sum_alpha / sum_weights).clamp(0.0, 1.0);
    let mut channels = [0_u8; 4];
    for channel in 0..3 {
        let unpremultiplied = (sum_premultiplied[channel] / sum_weights) / alpha.max(f32::EPSILON);
        channels[channel] = linear_to_srgb_byte(unpremultiplied);
    }
    channels[3] = (alpha * 255.0).round() as u8;
    Rgba(channels)
}

fn kernel_weight(distance: f32, filter: FilterType) -> f32 {
    let distance = distance.abs();
    match filter {
        FilterType::Triangle => (1.0 - distance).max(0.0),
        FilterType::CatmullRom => {
            // Catmull-Rom (a = -0.5) cubic.
            if distance < 1.0 {
                1.5 * distance.powi(3) - 2.5 * distance.powi(2) + 1.0
            } else if distance < 2.0 {
                -0.5 * distance.powi(3) + 2.5 * distance.powi(2) - 4.0 * distance + 2.0
            } else {
                0.0
            }
        }
        _ => {
            // Lanczos with a window of 3; the Gaussian resampler shares it.
            if distance < 3.0 {
                let pi_distance = std::f32::consts::PI * distance;
                let sinc = if distance < f32::EPSILON {
                    1.0
                } else {
                    let window = (pi_distance / 3.0).sin() / (pi_distance / 3.0);
                    (pi_distance.sin() / pi_distance) * window
                };
                sinc
            } else {
                0.0
            }
        }
    }
}
