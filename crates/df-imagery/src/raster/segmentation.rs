//! Fixed legacy focus-mask rasterization over an authorized segmentation observation.
//! Run projection and encoded Gray Gaussian blur preserve the existing display-mask bytes.

use image::{GrayImage, ImageReader, Luma, Rgba, RgbaImage};
use std::{io::Cursor, mem::size_of};

use super::{CancelCheck, RasterError, ResolveBytes, bounds::LiveBudget, decode};
use crate::{AnalysisMask, AnalysisMaskSet, ImageAnalysisResult, TransformSource};

pub(super) fn retained_bytes(result: &ImageAnalysisResult) -> Result<u64, RasterError> {
    let ImageAnalysisResult::Segmentation { masks, .. } = result else {
        return Err(RasterError::ResolveFailed);
    };
    let mut bytes = capacity_bytes::<AnalysisMaskSet>(masks.capacity())?;
    for set in masks {
        bytes = bytes
            .checked_add(capacity_bytes::<AnalysisMask>(set.instances.capacity())?)
            .ok_or(RasterError::ResolveFailed)?;
        for mask in &set.instances {
            bytes = bytes
                .checked_add(capacity_bytes::<crate::AnalysisMaskRun>(
                    mask.runs.capacity(),
                )?)
                .ok_or(RasterError::ResolveFailed)?;
        }
    }
    Ok(bytes)
}

fn capacity_bytes<T>(capacity: usize) -> Result<u64, RasterError> {
    capacity
        .checked_mul(size_of::<T>())
        .and_then(|bytes| u64::try_from(bytes).ok())
        .ok_or(RasterError::ResolveFailed)
}

pub(super) fn load(
    source: &TransformSource,
    result: &ImageAnalysisResult,
    resolve: ResolveBytes<'_>,
    budget: &LiveBudget,
    check_cancel: CancelCheck<'_>,
) -> Result<RgbaImage, RasterError> {
    let TransformSource::SegmentationMask {
        analyzed,
        prompt,
        mask_index,
        contract_revision,
        ..
    } = source
    else {
        return Err(RasterError::ResolveFailed);
    };
    let ImageAnalysisResult::Segmentation {
        source: result_source,
        contract_revision: result_revision,
        masks,
    } = result
    else {
        return Err(RasterError::ResolveFailed);
    };
    if analyzed != result_source || *contract_revision != 1 || result_revision != contract_revision
    {
        return Err(RasterError::ResolveFailed);
    }
    let mut selected_sets = masks.iter().filter(|set| set.prompt == *prompt);
    let set = selected_sets.next().ok_or(RasterError::ResolveFailed)?;
    if selected_sets.next().is_some() {
        return Err(RasterError::ResolveFailed);
    }
    let mut selected_masks = set
        .instances
        .iter()
        .filter(|mask| mask.index == *mask_index);
    let mask = selected_masks.next().ok_or(RasterError::ResolveFailed)?;
    if selected_masks.next().is_some() {
        return Err(RasterError::ResolveFailed);
    }
    validate_runs(set, mask)?;
    let bytes = decode::resolve_verified(resolve, analyzed, budget)?;
    let format = decode::sniff_format(&bytes)?;
    if decode::has_icc_profile(&bytes, format) {
        return Err(RasterError::UnsupportedColorProfile);
    }
    let (source_width_px, source_height_px) = ImageReader::with_format(Cursor::new(&bytes), format)
        .into_dimensions()
        .map_err(|_| RasterError::DecodeFailed)?;
    // Source pixels are never allocated: only their intrinsic dimensions enter the recipe.
    crate::Dimensions::new(source_width_px, source_height_px).map_err(|_| {
        RasterError::DecodedTooLarge {
            pixels: u64::from(source_width_px) * u64::from(source_height_px),
        }
    })?;
    drop(bytes);
    render(
        set,
        mask,
        source_width_px,
        source_height_px,
        budget,
        check_cancel,
    )
}

fn validate_runs(set: &AnalysisMaskSet, mask: &AnalysisMask) -> Result<(), RasterError> {
    if set.width_px == 0 || set.height_px == 0 || set.width_px > 8192 || set.height_px > 8192 {
        return Err(RasterError::ResolveFailed);
    }
    let pixels = u64::from(set.width_px) * u64::from(set.height_px);
    let mut end = 0;
    for run in &mask.runs {
        let start = u64::from(run.start);
        let next = start + u64::from(run.length);
        if run.length == 0 || start < end || next > pixels {
            return Err(RasterError::ResolveFailed);
        }
        end = next;
    }
    Ok(())
}

fn render(
    set: &AnalysisMaskSet,
    mask: &AnalysisMask,
    source_width_px: u32,
    source_height_px: u32,
    budget: &LiveBudget,
    check_cancel: CancelCheck<'_>,
) -> Result<RgbaImage, RasterError> {
    let scale = 768.0 / f64::from(source_width_px.max(source_height_px).max(768));
    // The fixed fit bounds both rounded dimensions by 768, including thin sources.
    let width_px = (f64::from(source_width_px) * scale).round().max(1.0) as u32;
    let height_px = (f64::from(source_height_px) * scale).round().max(1.0) as u32;
    budget.check_pixels(width_px, height_px)?;
    budget.check_retained(0, workspace_bytes(width_px, height_px))?;
    check_cancel()?;
    let mut alpha = GrayImage::new(width_px, height_px);
    let source_stride = u64::from(set.width_px);
    for run in &mask.runs {
        let mut start = u64::from(run.start);
        let end = start + u64::from(run.length);
        while start < end {
            check_cancel()?;
            let row = start / source_stride;
            let row_end = ((row + 1) * source_stride).min(end);
            let from = start % source_stride;
            let to = row_end - row * source_stride;
            for y in
                project(row, height_px, set.height_px)?..project(row + 1, height_px, set.height_px)?
            {
                for x in
                    project(from, width_px, set.width_px)?..project(to, width_px, set.width_px)?
                {
                    alpha.put_pixel(x, y, Luma([255]));
                }
            }
            start = row_end;
        }
    }
    check_cancel()?;
    let blurred = image::imageops::blur(&alpha, (height_px as f32 * 0.005).max(0.5));
    check_cancel()?;
    drop(alpha);
    let mut rgba = RgbaImage::new(width_px, height_px);
    for row in 0..height_px {
        check_cancel()?;
        for column in 0..width_px {
            rgba.put_pixel(
                column,
                row,
                Rgba([255, 255, 255, blurred.get_pixel(column, row)[0]]),
            );
        }
    }
    Ok(rgba)
}

fn project(pixel: u64, output_px: u32, input_px: u32) -> Result<u32, RasterError> {
    u32::try_from((pixel * u64::from(output_px) + u64::from(input_px / 2)) / u64::from(input_px))
        .map_err(|_| RasterError::ResolveFailed)
}

fn workspace_bytes(width_px: u32, height_px: u32) -> u64 {
    let pixels = u64::from(width_px) * u64::from(height_px);
    let sigma = (height_px as f32 * 0.005).max(0.5);
    // image0.25.10 sample.rs:1237 derives an odd kernel. Fixed sigma <=3.84 gives <=23 taps.
    let mut kernel = (((((sigma - 0.8) / 0.3) + 1.0) * 2.0) + 1.0).max(3.0) as u64;
    if kernel.is_multiple_of(2) {
        kernel += 1;
    }
    let pad = (kernel / 2).max(1);
    // sample.rs:1435..1532: source Gray P, two f32 planes 8P, result Gray P.
    // filter_1d.rs:696..724 keeps two original/two scanned kernels and one replacement;
    // kernel<55 selects its ring queue: f32 padded row + width*kernel ring + kernel row slices.
    let gray_and_float_planes = pixels * (1 + 4 + 4 + 1);
    let kernels = kernel * 5 * 4;
    let padded_row = (u64::from(width_px) + 2 * pad) * 4;
    let ring = u64::from(width_px) * kernel * 4;
    let row_slices = kernel * size_of::<&[f32]>() as u64;
    gray_and_float_planes + kernels + padded_row + ring + row_slices
}

#[cfg(test)]
mod tests;
