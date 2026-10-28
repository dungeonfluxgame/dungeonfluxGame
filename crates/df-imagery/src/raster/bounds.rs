//! Byte and pixel budgets checked before allocation, never after.
//!
//! Every bound uses widening or checked integer arithmetic so no product or
//! sum on this path can wrap. Decode sizes come from the container header;
//! intermediate buffers are reserved against the same ceiling before they
//! are allocated; encoded output is measured before it is returned.

use super::RasterError;
use crate::config::TransformLimits;
use crate::output::MAXIMUM_OUTPUT_PIXELS;
use crate::transform::Anchor1D;

/// Bytes per pixel of the canonical working representation.
pub(crate) const WORKING_BYTES_PER_PIXEL: u64 = 4;

/// Injected local limits plus the shared asset pixel ceiling.
pub(crate) struct LiveBudget {
    max_decoded_bytes: u64,
    max_output_bytes: u64,
}

impl LiveBudget {
    pub(crate) fn new(limits: &TransformLimits) -> Result<Self, RasterError> {
        if !limits.enabled {
            return Err(RasterError::TransformsDisabled);
        }
        Ok(Self {
            max_decoded_bytes: limits.max_decoded_bytes,
            max_output_bytes: limits.max_output_bytes,
        })
    }

    /// Maximum encoded output bytes; the bounded writer stops here, never
    /// after a post-hoc measurement.
    pub(crate) fn max_output_bytes(&self) -> u64 {
        self.max_output_bytes
    }

    /// Resident input bytes must fit the decoded budget before decoding.
    pub(crate) fn check_input_len(&self, len_bytes: u64) -> Result<(), RasterError> {
        if len_bytes > self.max_decoded_bytes {
            return Err(RasterError::InputTooLarge { len_bytes });
        }
        Ok(())
    }

    /// Header-claimed dimensions must fit the shared pixel ceiling and the
    /// decoded budget once widened to working bytes.
    pub(crate) fn check_pixels(&self, width_px: u32, height_px: u32) -> Result<(), RasterError> {
        let pixels = u64::from(width_px).checked_mul(u64::from(height_px));
        let pixels = pixels.unwrap_or(u64::MAX);
        if width_px == 0 || height_px == 0 || pixels > MAXIMUM_OUTPUT_PIXELS {
            return Err(RasterError::DecodedTooLarge { pixels });
        }
        let needed_bytes = pixels.saturating_mul(WORKING_BYTES_PER_PIXEL);
        if needed_bytes > self.max_decoded_bytes {
            return Err(RasterError::DecodedTooLarge { pixels });
        }
        Ok(())
    }

    /// A new buffer may be allocated only when the retained working set plus
    /// the newcomer still fits the decoded budget.
    pub(crate) fn check_retained(
        &self,
        retained_bytes: u64,
        additional_bytes: u64,
    ) -> Result<(), RasterError> {
        let needed_bytes = retained_bytes.saturating_add(additional_bytes);
        if needed_bytes > self.max_decoded_bytes {
            return Err(RasterError::LiveBudgetExceeded { needed_bytes });
        }
        Ok(())
    }

    /// Encoded output must fit the output budget before it is returned.
    pub(crate) fn check_output_len(&self, len_bytes: u64) -> Result<(), RasterError> {
        if len_bytes > self.max_output_bytes {
            return Err(RasterError::OutputTooLarge { len_bytes });
        }
        Ok(())
    }
}

/// Converts a checked pixel dimension to `f32` for sampling math. Callers
/// only pass dimensions already bound by `check_pixels`, so the value fits
/// the shared pixel ceiling and the conversion cannot wrap; `f64::from`
/// would be exact, but sampling needs `f32` and the bounded rounding is
/// far below one pixel.
pub(crate) fn dim_to_f32(value_px: u32) -> f32 {
    value_px as f32
}

/// Working bytes held by one RGBA raster of the given dimensions.
pub(crate) fn rgba_bytes(width_px: u32, height_px: u32) -> u64 {
    u64::from(width_px)
        .saturating_mul(u64::from(height_px))
        .saturating_mul(WORKING_BYTES_PER_PIXEL)
}

/// Start of content of `content_len_px` placed on a `canvas_len_px` axis.
/// Negative when the content overhangs and must be clipped by the caller.
pub(crate) fn anchor_start(
    canvas_len_px: u32,
    content_len_px: u32,
    axis: Anchor1D,
) -> i64 {
    match axis {
        Anchor1D::Start => 0,
        Anchor1D::Center => (i64::from(canvas_len_px) - i64::from(content_len_px)) / 2,
        Anchor1D::End => i64::from(canvas_len_px) - i64::from(content_len_px),
    }
}
