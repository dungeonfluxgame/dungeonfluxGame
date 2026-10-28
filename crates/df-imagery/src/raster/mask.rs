//! Scalar coverage application and ordered source-over composition.
//!
//! Masks are coverage, never content: the decoded mask asset or bound
//! coverage data reduces to one scalar per pixel through the declared channel,
//! then multiplies or replaces alpha. Layers composite in listed order with
//! their permille opacity; only source-over ships until another mode lands
//! with defined equations and fixtures.

use image::{GenericImageView, GrayImage, Luma, RgbaImage};

use super::RasterError;
use super::bounds::{self, LiveBudget};
use super::color::{linear_to_srgb_byte, luma_rec709, srgb_byte_to_linear};
use super::decode;
use super::{CancelCheck, ResolveBytes};
use crate::transform::{CompositeLayer, MaskChannel, MaskMode, MaskSource, MaskSpec};

/// Applies one mask step: decodes the coverage, feathers and inverts it per
/// the spec, aligns it over the raster, and folds it into alpha. Pixels the
/// mask does not cover keep their alpha.
pub(crate) fn apply_mask(
    mut image: RgbaImage,
    spec: &MaskSpec,
    resolve: ResolveBytes<'_>,
    budget: &LiveBudget,
    check_cancel: CancelCheck<'_>,
) -> Result<RgbaImage, RasterError> {
    let (width_px, height_px) = image.dimensions();
    let expected_width_px = spec.width_px();
    let expected_height_px = spec.height_px();
    let mask_bytes = match spec.source() {
        MaskSource::Asset(pin) => decode::resolve_verified(resolve, pin, budget)?,
        MaskSource::Coverage(coverage) => {
            decode::resolve_verified(resolve, &coverage.coverage, budget)?
        }
    };
    let mask_image = decode::decode_bounded(&mask_bytes, budget)?;
    let (mask_width_px, mask_height_px) = mask_image.dimensions();
    if mask_width_px != expected_width_px || mask_height_px != expected_height_px {
        return Err(RasterError::DimensionMismatch {
            actual_width_px: mask_width_px,
            actual_height_px: mask_height_px,
            expected_width_px,
            expected_height_px,
        });
    }
    let field_len = (mask_width_px as usize) * (mask_height_px as usize);
    budget.check_retained(
        bounds::rgba_bytes(width_px, height_px),
        (field_len as u64).saturating_mul(4),
    )?;
    let mut coverage = scalar_coverage(&mask_image, spec.channel());
    let feather_px = spec.feather_radius_px();
    if feather_px > 0.0 {
        coverage = feather_field(
            &coverage,
            mask_width_px,
            mask_height_px,
            feather_px,
            budget,
            check_cancel,
        )?;
    }
    if spec.invert() {
        for value in &mut coverage {
            *value = 1.0 - *value;
        }
    }
    let start_x_px = bounds::anchor_start(width_px, mask_width_px, spec.align().x);
    let start_y_px = bounds::anchor_start(height_px, mask_height_px, spec.align().y);
    for row in 0..height_px {
        if row % 16 == 0 {
            check_cancel()?;
        }
        let mask_y = i64::from(row) - start_y_px;
        for col in 0..width_px {
            let mask_x = i64::from(col) - start_x_px;
            if mask_x < 0
                || mask_y < 0
                || mask_x >= i64::from(mask_width_px)
                || mask_y >= i64::from(mask_height_px)
            {
                continue;
            }
            let cover = coverage[mask_y as usize * mask_width_px as usize + mask_x as usize]
                .clamp(0.0, 1.0);
            let mut pixel = *image.get_pixel(col, row);
            let alpha = f32::from(pixel[3]) / 255.0;
            pixel[3] = match spec.mode() {
                MaskMode::Multiply => ((alpha * cover).clamp(0.0, 1.0) * 255.0).round() as u8,
                MaskMode::Replace => (cover * 255.0).round() as u8,
            };
            image.put_pixel(col, row, pixel);
        }
    }
    Ok(image)
}

/// Decodes one coverage source to a scalar field: the alpha channel when the
/// decoded bytes carry transparency. `Alpha` reads alpha; `Luminance`
/// always reads Rec.709 luma, even for transparent inputs.
fn scalar_coverage(image: &RgbaImage, channel: MaskChannel) -> Vec<f32> {
    let (width_px, height_px) = image.dimensions();
    let mut coverage = Vec::with_capacity(width_px as usize * height_px as usize);
    for pixel in image.pixels() {
        let value = match channel {
            MaskChannel::Alpha => f32::from(pixel[3]) / 255.0,
            MaskChannel::Luminance => luma_rec709(
                f32::from(pixel[0]) / 255.0,
                f32::from(pixel[1]) / 255.0,
                f32::from(pixel[2]) / 255.0,
            ),
        };
        coverage.push(value);
    }
    coverage
}

fn feather_field(
    field: &[f32],
    width_px: u32,
    height_px: u32,
    radius_px: f32,
    budget: &LiveBudget,
    check_cancel: CancelCheck<'_>,
) -> Result<Vec<f32>, RasterError> {
    let pixels = u64::from(width_px).saturating_mul(u64::from(height_px));
    // Scalar field (4 bytes per pixel) plus gray input, blurred gray, and
    // feathered float output (6 bytes per pixel, rounded to 8); the pinned
    // library blur below is bounded by these same dimensions and checked
    // here because it cannot be cancelled mid-call.
    budget.check_retained(
        (field.len() as u64).saturating_mul(4),
        pixels.saturating_mul(8),
    )?;
    // Feathering is always Gaussian with sigma equal to the radius; the
    // spec carries no kernel choice, so there is nothing else to honor.
    check_cancel()?;
    let gray = GrayImage::from_fn(width_px, height_px, |x, y| {
        let value = field[y as usize * width_px as usize + x as usize].clamp(0.0, 1.0);
        Luma([(value * 255.0).round() as u8])
    });
    check_cancel()?;
    let blurred = image::imageops::blur(&gray, radius_px);
    let mut feathered = Vec::with_capacity(field.len());
    for pixel in blurred.pixels() {
        feathered.push(f32::from(pixel[0]) / 255.0);
    }
    check_cancel()?;
    Ok(feathered)
}

/// Composites ordered layers over the raster with source-over and the layer
/// permille opacity. Layers outside the raster are clipped, never an error.
pub(crate) fn apply_composite(
    mut image: RgbaImage,
    layers: &[CompositeLayer],
    resolve: ResolveBytes<'_>,
    budget: &LiveBudget,
    check_cancel: CancelCheck<'_>,
) -> Result<RgbaImage, RasterError> {
    let (width_px, height_px) = image.dimensions();
    for layer in layers {
        check_cancel()?;
        let layer_bytes = decode::resolve_verified(resolve, layer.source(), budget)?;
        let layer_image = decode::decode_bounded(&layer_bytes, budget)?;
        budget.check_retained(
            bounds::rgba_bytes(width_px, height_px),
            bounds::rgba_bytes(layer_image.width(), layer_image.height()),
        )?;
        composite_layer(&mut image, &layer_image, layer, check_cancel)?;
    }
    Ok(image)
}

fn composite_layer(
    image: &mut RgbaImage,
    layer: &RgbaImage,
    spec: &CompositeLayer,
    check_cancel: CancelCheck<'_>,
) -> Result<(), RasterError> {
    let (width_px, height_px) = image.dimensions();
    let (layer_width_px, layer_height_px) = layer.dimensions();
    let opacity = f32::from(spec.opacity_permille()) / 1000.0;
    if opacity <= 0.0 {
        return Ok(());
    }
    for row in 0..layer_height_px {
        if row % 16 == 0 {
            check_cancel()?;
        }
        let target_y = i64::from(spec.y_px()) + i64::from(row);
        if target_y < 0 || target_y >= i64::from(height_px) {
            continue;
        }
        for col in 0..layer_width_px {
            let target_x = i64::from(spec.x_px()) + i64::from(col);
            if target_x < 0 || target_x >= i64::from(width_px) {
                continue;
            }
            let top = layer.get_pixel(col, row);
            let layer_alpha = (f32::from(top[3]) / 255.0 * opacity).clamp(0.0, 1.0);
            if layer_alpha <= 0.0 {
                continue;
            }
            let bottom = image.get_pixel(target_x as u32, target_y as u32);
            let bottom_alpha = f32::from(bottom[3]) / 255.0;
            let out_alpha = layer_alpha + bottom_alpha * (1.0 - layer_alpha);
            if out_alpha <= 0.0 {
                continue;
            }
            // Source-over in linear light with premultiplied alpha so
            // transparent edges do not fringe; encoded mixing would halo.
            let mut out = [0_u8; 4];
            for channel in 0..3 {
                let top_c = srgb_byte_to_linear(top[channel]);
                let bottom_c = srgb_byte_to_linear(bottom[channel]);
                let mixed = (top_c * layer_alpha
                    + bottom_c * bottom_alpha * (1.0 - layer_alpha))
                    / out_alpha.max(f32::EPSILON);
                out[channel] = linear_to_srgb_byte(mixed);
            }
            out[3] = (out_alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
            image.put_pixel(target_x as u32, target_y as u32, image::Rgba(out));
        }
    }
    Ok(())
}
