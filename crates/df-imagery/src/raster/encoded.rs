//! Versioned compatibility operations over encoded straight-alpha bytes.
//!
//! The pinned image 0.25.10 algorithms reproduce pre-migration consumers;
//! standard geometry and color operations retain linear-light semantics.
//! Admission includes the library resizer's floating-point intermediate and
//! weight-vector growth. Library sampling is synchronous, checked before and
//! after the call; matte and alpha loops also poll between rows.

use image::{DynamicImage, Pixel, Rgba, RgbaImage, imageops};

use super::bounds::{LiveBudget, rgba_bytes};
use super::{CancelCheck, RasterError};
use crate::output::{Dimensions, Rgb};
use crate::transform::{EncodedAlphaBlend, EncodedFilter};

pub(super) fn resize(
    image: RgbaImage,
    dimensions: Dimensions,
    filter: EncodedFilter,
    budget: &LiveBudget,
    check_cancel: CancelCheck<'_>,
) -> Result<RgbaImage, RasterError> {
    let width_px = dimensions.width_px();
    let height_px = dimensions.height_px();
    budget.check_pixels(width_px, height_px)?;
    let retained_bytes = rgba_bytes(image.width(), image.height());
    let output_bytes = rgba_bytes(width_px, height_px);
    let workspace_bytes = resize_workspace_bytes(image.width(), image.height(), height_px);
    let additional_bytes = output_bytes.saturating_add(workspace_bytes);
    let needed_bytes = retained_bytes.saturating_add(additional_bytes);
    if isize::try_from(needed_bytes).is_err() {
        return Err(RasterError::LiveBudgetExceeded { needed_bytes });
    }
    budget.check_retained(retained_bytes, additional_bytes)?;
    check_cancel()?;
    let filter = match filter {
        EncodedFilter::Triangle => imageops::FilterType::Triangle,
        EncodedFilter::Lanczos3 => imageops::FilterType::Lanczos3,
    };
    let resized = imageops::resize(&image, width_px, height_px, filter);
    check_cancel()?;
    Ok(resized)
}

fn resize_workspace_bytes(width_px: u32, height_px: u32, target_height_px: u32) -> u64 {
    // imageops/sample.rs:490-508,1005-1007 retains an RGBA-f32 vertical
    // intermediate (16 bytes/pixel) while horizontal sampling creates output.
    let intermediate_bytes = u64::from(width_px)
        .saturating_mul(u64::from(target_height_px))
        .saturating_mul(16);
    // Both sampling weight vectors push <=one f32/source-axis pixel. The
    // Rust 1.98.1 alloc/raw_vec/mod.rs:141-154,465-491 doubles capacity with
    // a minimum of four f32 values;
    // old+new growth requests fit 3*max(axis,4)*sizeof(f32). Their actual
    // lifetimes do not overlap, but counting both is conservative.
    let weights_bytes = u64::from(width_px.max(4))
        .saturating_add(u64::from(height_px.max(4)))
        .saturating_mul(3 * 4);
    intermediate_bytes.saturating_add(weights_bytes)
}

pub(super) fn thumbnail(
    image: RgbaImage,
    bounds: Dimensions,
    budget: &LiveBudget,
    check_cancel: CancelCheck<'_>,
) -> Result<RgbaImage, RasterError> {
    // imageops/sample.rs:613-695 allocates only the output; the aspect-fit
    // thumbnail is bounded by these constructor-checked dimensions.
    budget.check_pixels(bounds.width_px(), bounds.height_px())?;
    budget.check_retained(
        rgba_bytes(image.width(), image.height()),
        rgba_bytes(bounds.width_px(), bounds.height_px()),
    )?;
    check_cancel()?;
    let resized = DynamicImage::ImageRgba8(image)
        .thumbnail(bounds.width_px(), bounds.height_px())
        .into_rgba8();
    check_cancel()?;
    Ok(resized)
}

pub(super) fn matte(
    mut image: RgbaImage,
    fill: Rgb,
    blend: EncodedAlphaBlend,
    check_cancel: CancelCheck<'_>,
) -> Result<RgbaImage, RasterError> {
    check_cancel()?;
    for row in 0..image.height() {
        if row % 16 == 0 {
            check_cancel()?;
        }
        for column in 0..image.width() {
            let pixel = image.get_pixel_mut(column, row);
            let mut opaque = Rgba([fill.red, fill.green, fill.blue, 255]);
            match blend {
                EncodedAlphaBlend::IntegerFloor => {
                    let alpha = u16::from(pixel[3]);
                    for channel in 0..3 {
                        let mixed = (u16::from(pixel[channel]) * alpha
                            + u16::from(opaque[channel]) * (255 - alpha))
                            / 255;
                        // Convex u8 combination divided by 255 always fits u8.
                        opaque[channel] = mixed as u8;
                    }
                }
                EncodedAlphaBlend::ImageSourceOver => opaque.blend(pixel),
            }
            *pixel = opaque;
        }
    }
    check_cancel()?;
    Ok(image)
}

pub(super) fn discard_alpha(
    mut image: RgbaImage,
    check_cancel: CancelCheck<'_>,
) -> Result<RgbaImage, RasterError> {
    check_cancel()?;
    for row in 0..image.height() {
        if row % 16 == 0 {
            check_cancel()?;
        }
        for column in 0..image.width() {
            image.get_pixel_mut(column, row)[3] = 255;
        }
    }
    check_cancel()?;
    Ok(image)
}

#[cfg(test)]
mod tests;
