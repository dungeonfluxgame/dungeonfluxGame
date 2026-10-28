//! Photometric effects in the declared working space.
//!
//! Revision `RASTER_COLOR_REV` freezes the transfer, luminance, and hue
//! equations below: normalized sRGB with straight alpha. Color steps never
//! touch alpha; resampling and compositing own their own alpha handling.

use image::RgbaImage;

use super::RasterError;
use super::bounds::{self, LiveBudget};
use super::CancelCheck;
use crate::transform::{BlurAlgorithm, BlurSpec, ColorEffect, ExposureStops, SharpenSpec};

/// Version tag of the color equations in this module; a change that alters
/// output bytes requires a new revision in the recipe key. Revision 2 moves
/// blur, sharpen detail, composite, matte, resampling, and rotation sampling
/// to linear light with premultiplied alpha; color steps keep revision 1
/// equations.
pub(crate) const RASTER_COLOR_REV: u32 = 2;

/// Rec.709 luma weights used for saturation, grayscale, and luminance masks.
pub(crate) fn luma_rec709(red: f32, green: f32, blue: f32) -> f32 {
    0.2126 * red + 0.7152 * green + 0.0722 * blue
}

/// Linear-light transfer for resampling, blur, composite, and matte. Color
/// steps keep their frozen sRGB equations; these helpers give the linear
/// profile its own conversion without touching that revision.
pub(crate) fn srgb_byte_to_linear(byte: u8) -> f32 {
    srgb_to_linear(f32::from(byte) / 255.0)
}

pub(crate) fn linear_to_srgb_byte(value: f32) -> u8 {
    encode_channel(linear_to_srgb(value))
}

fn srgb_to_linear(component: f32) -> f32 {
    if component <= 0.04045 {
        component / 12.92
    } else {
        ((component + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(component: f32) -> f32 {
    let clamped = component.clamp(0.0, 1.0);
    if clamped <= 0.0031308 {
        clamped * 12.92
    } else {
        1.055 * clamped.powf(1.0 / 2.4) - 0.055
    }
}

fn encode_channel(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Adds the signed normalized offset in working RGB space, then clamps at
/// the output boundary. Neutral is zero; alpha is preserved.
pub(crate) fn apply_brightness(
    mut image: RgbaImage,
    offset: crate::transform::BrightnessOffset,
    check_cancel: CancelCheck<'_>,
) -> Result<RgbaImage, RasterError> {
    if offset.is_neutral() {
        return Ok(image);
    }
    let shift = offset.value();
    for (index, pixel) in image.pixels_mut().enumerate() {
        if index % 4096 == 0 {
            check_cancel()?;
        }
        for channel in 0..3 {
            let normalized = f32::from(pixel[channel]) / 255.0 + shift;
            pixel[channel] = encode_channel(normalized);
        }
    }
    Ok(image)
}

/// Multiplies linear-light RGB by `2^stops`; neutral is zero stops. Black
/// stays black at any stops; unrepresentable gains clamp to white.
pub(crate) fn apply_exposure(
    mut image: RgbaImage,
    stops: ExposureStops,
    check_cancel: CancelCheck<'_>,
) -> Result<RgbaImage, RasterError> {
    if stops.is_neutral() {
        return Ok(image);
    }
    let gain = 2.0_f32.powf(stops.value());
    for (index, pixel) in image.pixels_mut().enumerate() {
        if index % 4096 == 0 {
            check_cancel()?;
        }
        for channel in 0..3 {
            let linear = srgb_to_linear(f32::from(pixel[channel]) / 255.0);
            let scaled = linear * gain;
            let finite = if scaled.is_finite() { scaled } else { 1.0 };
            pixel[channel] = encode_channel(linear_to_srgb(finite));
        }
    }
    Ok(image)
}

/// Contrast pivots around the explicit point, saturation mixes toward luma,
/// hue rotates the YIQ chroma plane, and grayscale is luma on all channels.
pub(crate) fn apply_color_effect(
    mut image: RgbaImage,
    effect: ColorEffect,
    check_cancel: CancelCheck<'_>,
) -> Result<RgbaImage, RasterError> {
    match effect {
        ColorEffect::Contrast { gain, pivot } => {
            for (index, pixel) in image.pixels_mut().enumerate() {
                if index % 4096 == 0 {
                    check_cancel()?;
                }
                for channel in 0..3 {
                    let normalized = f32::from(pixel[channel]) / 255.0;
                    pixel[channel] = encode_channel((normalized - pivot) * gain + pivot);
                }
            }
        }
        ColorEffect::Saturation { factor } => {
            for (index, pixel) in image.pixels_mut().enumerate() {
                if index % 4096 == 0 {
                    check_cancel()?;
                }
                let channels = [
                    f32::from(pixel[0]) / 255.0,
                    f32::from(pixel[1]) / 255.0,
                    f32::from(pixel[2]) / 255.0,
                ];
                let luma = luma_rec709(channels[0], channels[1], channels[2]);
                for channel in 0..3 {
                    pixel[channel] = encode_channel(luma + factor * (channels[channel] - luma));
                }
            }
        }
        ColorEffect::HueShift { degrees } => {
            // Forward and inverse YIQ matrices with the chroma plane rotated
            // by the requested hue angle.
            let radians = degrees.to_radians();
            let (cosine, sine) = (radians.cos(), radians.sin());
            for (index, pixel) in image.pixels_mut().enumerate() {
                if index % 4096 == 0 {
                    check_cancel()?;
                }
                let red = f32::from(pixel[0]) / 255.0;
                let green = f32::from(pixel[1]) / 255.0;
                let blue = f32::from(pixel[2]) / 255.0;
                let intensity = 0.299 * red + 0.587 * green + 0.114 * blue;
                let chroma_i = 0.596 * red - 0.274 * green - 0.322 * blue;
                let chroma_q = 0.211 * red - 0.523 * green + 0.312 * blue;
                let rotated_i = cosine * chroma_i - sine * chroma_q;
                let rotated_q = sine * chroma_i + cosine * chroma_q;
                pixel[0] = encode_channel(intensity + 0.956 * rotated_i + 0.621 * rotated_q);
                pixel[1] = encode_channel(intensity - 0.272 * rotated_i - 0.647 * rotated_q);
                pixel[2] = encode_channel(intensity - 1.106 * rotated_i + 1.703 * rotated_q);
            }
        }
        ColorEffect::Grayscale => {
            for (index, pixel) in image.pixels_mut().enumerate() {
                if index % 4096 == 0 {
                    check_cancel()?;
                }
                let luma = luma_rec709(
                    f32::from(pixel[0]) / 255.0,
                    f32::from(pixel[1]) / 255.0,
                    f32::from(pixel[2]) / 255.0,
                );
                let gray = encode_channel(luma);
                pixel[0] = gray;
                pixel[1] = gray;
                pixel[2] = gray;
            }
        }
    }
    Ok(image)
}

/// Softens with the named kernel at the current-raster radius; edges clamp.
/// Gaussian delegates to the pinned blur with sigma equal to the radius.
pub(crate) fn apply_blur(
    image: RgbaImage,
    spec: &BlurSpec,
    budget: &LiveBudget,
    check_cancel: CancelCheck<'_>,
) -> Result<RgbaImage, RasterError> {
    blur_rgba(&image, spec.radius_px(), spec.algorithm(), budget, check_cancel)
}

/// Unsharp mask: detail above the noise threshold is pushed away from the
/// blurred copy by the amount factor. Zero amount is the identity. Detail is
/// measured in linear light so halos follow light, not encoded values.
pub(crate) fn apply_sharpen(
    image: RgbaImage,
    spec: &SharpenSpec,
    budget: &LiveBudget,
    check_cancel: CancelCheck<'_>,
) -> Result<RgbaImage, RasterError> {
    if spec.amount_factor() == 0.0 {
        return Ok(image);
    }
    let blurred = blur_rgba(
        &image,
        spec.radius_px(),
        BlurAlgorithm::Gaussian,
        budget,
        check_cancel,
    )?;
    use image::GenericImageView as _;
    let (width_px, height_px) = image.dimensions();
    budget.check_retained(
        bounds::rgba_bytes(width_px, height_px).saturating_mul(2),
        bounds::rgba_bytes(width_px, height_px),
    )?;
    let threshold = f32::from(spec.threshold_0_255()) / 255.0;
    let amount = spec.amount_factor();
    let mut sharpened = image;
    for (index, (pixel, blur_pixel)) in
        sharpened.pixels_mut().zip(blurred.pixels()).enumerate()
    {
        if index % 4096 == 0 {
            check_cancel()?;
        }
        for channel in 0..3 {
            let original = srgb_byte_to_linear(pixel[channel]);
            let soft = srgb_byte_to_linear(blur_pixel[channel]);
            let detail = original - soft;
            if detail.abs() > threshold {
                pixel[channel] = linear_to_srgb_byte(original + amount * detail);
            }
        }
    }
    Ok(sharpened)
}

fn blur_rgba(
    image: &RgbaImage,
    radius_px: f32,
    algorithm: BlurAlgorithm,
    budget: &LiveBudget,
    check_cancel: CancelCheck<'_>,
) -> Result<RgbaImage, RasterError> {
    use image::GenericImageView as _;
    let (width_px, height_px) = image.dimensions();
    let pixels = u64::from(width_px).saturating_mul(u64::from(height_px));
    // Box holds horizontal float sums (16 bytes per pixel) plus u32 window
    // widths (4 bytes per pixel) plus the output raster (4 bytes per pixel);
    // Gaussian holds one float row buffer (16 bytes per pixel) plus output.
    // Both reserve 24 bytes per pixel so the peak is checked before any
    // allocation, covering live decode, copies, and float buffers.
    let additional = pixels.saturating_mul(24);
    budget.check_retained(bounds::rgba_bytes(width_px, height_px), additional)?;
    check_cancel()?;
    let blurred = match algorithm {
        BlurAlgorithm::Gaussian => {
            gaussian_blur_linear(image, radius_px, budget, check_cancel)?
        }
        BlurAlgorithm::Box => {
            let radius = radius_px.ceil().max(1.0) as u32;
            box_blur_linear(image, radius, check_cancel)?
        }
    };
    check_cancel()?;
    Ok(blurred)
}

/// Separable box blur in linear light with premultiplied alpha and clamped
/// edges; the radius is in current-raster pixels and every row yields to the
/// cancellation check. Fully transparent neighborhoods stay transparent.
fn box_blur_linear(
    image: &RgbaImage,
    radius_px: u32,
    check_cancel: CancelCheck<'_>,
) -> Result<RgbaImage, RasterError> {
    use image::GenericImageView as _;
    let (width_px, height_px) = image.dimensions();
    let width_len = width_px as usize;
    let height_len = height_px as usize;
    let radius_len = radius_px as usize;
    // Horizontal pass: unnormalized premultiplied-linear sums plus the
    // window width, so the vertical pass weights edge windows exactly.
    let mut horizontal_sums = vec![[0.0_f32; 4]; width_len * height_len];
    let mut horizontal_counts = vec![0_u32; width_len * height_len];
    for row in 0..height_len {
        if row % 16 == 0 {
            check_cancel()?;
        }
        for col in 0..width_len {
            let left = col.saturating_sub(radius_len);
            let right = (col + radius_len).min(width_len - 1);
            let mut sums = [0.0_f32; 4];
            for sample in left..=right {
                let pixel = image.get_pixel(sample as u32, row as u32);
                let alpha = f32::from(pixel[3]) / 255.0;
                for channel in 0..3 {
                    sums[channel] += srgb_byte_to_linear(pixel[channel]) * alpha;
                }
                sums[3] += alpha;
            }
            horizontal_sums[row * width_len + col] = sums;
            horizontal_counts[row * width_len + col] = (right - left + 1) as u32;
        }
    }
    let mut blurred = RgbaImage::new(width_px, height_px);
    for row in 0..height_len {
        if row % 16 == 0 {
            check_cancel()?;
        }
        for col in 0..width_len {
            let top = row.saturating_sub(radius_len);
            let bottom = (row + radius_len).min(height_len - 1);
            let mut sums = [0.0_f32; 4];
            let mut count = 0_u32;
            for sample in top..=bottom {
                let cell = horizontal_sums[sample * width_len + col];
                for channel in 0..4 {
                    sums[channel] += cell[channel];
                }
                count += horizontal_counts[sample * width_len + col];
            }
            // Window counts stay within the checked pixel ceiling, so the
            // conversion cannot wrap; sub-pixel rounding is irrelevant here.
            let divisor = count.max(1) as f32;
            let avg_alpha = (sums[3] / divisor).clamp(0.0, 1.0);
            let mut out = [0_u8; 4];
            if avg_alpha <= 0.0 {
                out = [0, 0, 0, 0];
            } else {
                for channel in 0..3 {
                    let unpremultiplied = (sums[channel] / divisor) / avg_alpha.max(f32::EPSILON);
                    out[channel] = linear_to_srgb_byte(unpremultiplied);
                }
                out[3] = (avg_alpha * 255.0).round().clamp(0.0, 255.0) as u8;
            }
            blurred.put_pixel(col as u32, row as u32, image::Rgba(out));
        }
    }
    Ok(blurred)
}

/// Separable Gaussian blur in linear light with premultiplied alpha. Sigma
/// equals the requested radius; the kernel spans three sigma with clamped
/// edges. The bounded horizontal float buffer is reserved by the caller.
fn gaussian_blur_linear(
    image: &RgbaImage,
    radius_px: f32,
    budget: &LiveBudget,
    check_cancel: CancelCheck<'_>,
) -> Result<RgbaImage, RasterError> {
    use image::GenericImageView as _;
    let (width_px, height_px) = image.dimensions();
    let width_len = width_px as usize;
    let height_len = height_px as usize;
    let sigma = radius_px.max(0.5);
    let kernel_radius = (3.0 * sigma).ceil().max(1.0) as usize;
    // The 1D kernel is tiny next to the image, but a huge sigma must not
    // allocate before the operator byte budget refuses it.
    let kernel_len = kernel_radius.saturating_mul(2).saturating_add(1);
    let pixels = u64::from(width_px).saturating_mul(u64::from(height_px));
    budget.check_retained(
        pixels.saturating_mul(28),
        (kernel_len as u64).saturating_mul(4),
    )?;
    check_cancel()?;
    let mut kernel = Vec::with_capacity(kernel_radius * 2 + 1);
    let mut kernel_sum = 0.0_f32;
    for offset in -(kernel_radius as i64)..=(kernel_radius as i64) {
        let distance = offset as f32;
        let weight = (-distance * distance / (2.0 * sigma * sigma)).exp();
        kernel.push(weight);
        kernel_sum += weight;
    }
    for weight in &mut kernel {
        *weight /= kernel_sum.max(f32::EPSILON);
    }
    let mut horizontal = vec![[0.0_f32; 4]; width_len * height_len];
    for row in 0..height_len {
        if row % 16 == 0 {
            check_cancel()?;
        }
        for col in 0..width_len {
            let mut sums = [0.0_f32; 4];
            for (index, weight) in kernel.iter().enumerate() {
                let offset = index as i64 - kernel_radius as i64;
                let sample = (col as i64 + offset).clamp(0, width_len as i64 - 1) as u32;
                let pixel = image.get_pixel(sample, row as u32);
                let alpha = f32::from(pixel[3]) / 255.0;
                for channel in 0..3 {
                    sums[channel] += srgb_byte_to_linear(pixel[channel]) * alpha * *weight;
                }
                sums[3] += alpha * *weight;
            }
            horizontal[row * width_len + col] = sums;
        }
    }
    let mut blurred = RgbaImage::new(width_px, height_px);
    for row in 0..height_len {
        if row % 16 == 0 {
            check_cancel()?;
        }
        for col in 0..width_len {
            let mut sums = [0.0_f32; 4];
            for (index, weight) in kernel.iter().enumerate() {
                let offset = index as i64 - kernel_radius as i64;
                let sample = (row as i64 + offset).clamp(0, height_len as i64 - 1) as usize;
                let cell = horizontal[sample * width_len + col];
                for channel in 0..4 {
                    sums[channel] += cell[channel] * *weight;
                }
            }
            let alpha = sums[3].clamp(0.0, 1.0);
            let mut out = [0_u8; 4];
            if alpha <= 0.0 {
                out = [0, 0, 0, 0];
            } else {
                for channel in 0..3 {
                    out[channel] = linear_to_srgb_byte(sums[channel] / alpha.max(f32::EPSILON));
                }
                out[3] = (alpha * 255.0).round().clamp(0.0, 255.0) as u8;
            }
            blurred.put_pixel(col as u32, row as u32, image::Rgba(out));
        }
    }
    Ok(blurred)
}
