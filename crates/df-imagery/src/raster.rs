//! Private synchronous CPU raster executor for accepted transform recipes.
//!
//! Owns ordered application of [`TransformSpec`] steps plus the final
//! [`RasterOutputSpec`] encode on the calling thread. It never touches the
//! network, the filesystem, providers, credentials, publication, or ambient
//! clocks: byte access arrives through the injected resolver, cancellation
//! through the injected check, and time through the caller's own deadline.
//!
//! How it fails: typed [`RasterError`] names the first refusal; pixel and
//! byte bounds are checked before allocation; digests are verified before
//! decode; EXIF orientation is normalized once before coordinates apply.

mod bounds;
pub(crate) mod color;
pub(crate) mod decode;
mod encode;
mod encoded;
mod geometry;
mod mask;
mod segmentation;
pub(crate) mod webp_lossy;

use image::RgbaImage;

use crate::config::TransformLimits;
use crate::output::RasterOutputSpec;
use crate::request::{RequestLimits, SourceRef};
use crate::transform::{ImageTransform, TransformSource, TransformSpec};

/// Immutable bytes for one authorized pin; the service authorizes, this
/// executor only verifies the digest the pin already carries.
pub(crate) type ResolveBytes<'a> = &'a dyn Fn(&SourceRef) -> Result<Vec<u8>, RasterError>;

/// Cooperative cancellation or deadline check; the caller maps its own clock.
/// Polled between stages and across long pixel rows; never spawned or awaited.
pub(crate) type CancelCheck<'a> = &'a dyn Fn() -> Result<(), RasterError>;

/// Delivered raster bytes with the facts the service publishes.
pub(crate) struct RasterOutput {
    pub(crate) bytes: Vec<u8>,
    pub(crate) mime: &'static str,
    pub(crate) width_px: u32,
    pub(crate) height_px: u32,
    pub(crate) has_alpha: bool,
}

impl std::fmt::Debug for RasterOutput {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RasterOutput")
            .field("len_bytes", &self.bytes.len())
            .field("mime", &self.mime)
            .field("width_px", &self.width_px)
            .field("height_px", &self.height_px)
            .field("has_alpha", &self.has_alpha)
            .finish()
    }
}

/// Why a local transform was refused; callers branch on the variant, never on
/// text. No variant carries bytes, prompts, or credentials.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RasterError {
    TransformsDisabled,
    InvalidRecipe(crate::transform::TransformError),
    InvalidOutput(crate::output::OutputError),
    /// Input bytes are not PNG, JPEG, or WebP.
    UnsupportedFormat,
    /// Embedded ICC profile is present but no converter is pinned; the
    /// executor refuses instead of silently discarding the profile.
    UnsupportedColorProfile,
    DecodeFailed,
    EncodeFailed,
    DigestMismatch,
    ResolveFailed,
    InputTooLarge {
        len_bytes: u64,
    },
    DecodedTooLarge {
        pixels: u64,
    },
    LiveBudgetExceeded {
        needed_bytes: u64,
    },
    OutputTooLarge {
        len_bytes: u64,
    },
    CropOutOfBounds,
    DimensionMismatch {
        actual_width_px: u32,
        actual_height_px: u32,
        expected_width_px: u32,
        expected_height_px: u32,
    },
    Cancelled,
}

impl std::fmt::Display for RasterError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TransformsDisabled => formatter.write_str("local transforms are disabled"),
            Self::InvalidRecipe(error) => {
                write!(formatter, "the transform recipe is invalid: {error}")
            }
            Self::InvalidOutput(error) => {
                write!(formatter, "the output contract is invalid: {error}")
            }
            Self::UnsupportedFormat => formatter.write_str("the input is not png, jpeg or webp"),
            Self::UnsupportedColorProfile => {
                formatter.write_str("the input color profile is not supported")
            }
            Self::DecodeFailed => formatter.write_str("the input could not be decoded"),
            Self::EncodeFailed => formatter.write_str("the output could not be encoded"),
            Self::DigestMismatch => formatter.write_str("the input digest does not match"),
            Self::ResolveFailed => formatter.write_str("an input could not be resolved"),
            Self::InputTooLarge { len_bytes } => {
                write!(formatter, "the input is too large at {len_bytes} bytes")
            }
            Self::DecodedTooLarge { pixels } => write!(
                formatter,
                "the decoded input has too many pixels ({pixels})"
            ),
            Self::LiveBudgetExceeded { needed_bytes } => write!(
                formatter,
                "the memory budget cannot hold {needed_bytes} more bytes"
            ),
            Self::OutputTooLarge { len_bytes } => {
                write!(formatter, "the output is too large at {len_bytes} bytes")
            }
            Self::CropOutOfBounds => formatter.write_str("the crop is out of bounds"),
            Self::DimensionMismatch { .. } => {
                formatter.write_str("the output dimensions do not match")
            }
            Self::Cancelled => formatter.write_str("the transform was cancelled"),
        }
    }
}

impl std::error::Error for RasterError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidRecipe(error) => Some(error),
            Self::InvalidOutput(error) => Some(error),
            _ => None,
        }
    }
}

/// Runs the ordered recipe against authorized bytes and encodes one variant.
/// Pure CPU on the calling thread; the later bounded worker owns placement.
pub(crate) fn execute_transform(
    spec: &TransformSpec,
    output: &RasterOutputSpec,
    request_limits: &RequestLimits,
    transform_limits: &TransformLimits,
    resolve: ResolveBytes<'_>,
    check_cancel: CancelCheck<'_>,
) -> Result<RasterOutput, RasterError> {
    let budget = bounds::LiveBudget::new(transform_limits)?;
    spec.validate(request_limits)
        .map_err(RasterError::InvalidRecipe)?;
    output.validate().map_err(RasterError::InvalidOutput)?;
    check_cancel()?;
    let image = load_source(&spec.source, resolve, &budget)?;
    finish(image, spec, output, resolve, &budget, check_cancel)
}

/// Executes only after the runtime resolves and authorizes the actual completed analysis handle.
pub(crate) fn execute_segmentation_mask(
    spec: &TransformSpec,
    output: &RasterOutputSpec,
    request_limits: &RequestLimits,
    transform_limits: &TransformLimits,
    analysis: &crate::ImageAnalysisResult,
    resolve: ResolveBytes<'_>,
    check_cancel: CancelCheck<'_>,
) -> Result<RasterOutput, RasterError> {
    spec.validate(request_limits)
        .map_err(RasterError::InvalidRecipe)?;
    output.validate().map_err(RasterError::InvalidOutput)?;
    let retained_bytes = segmentation::retained_bytes(analysis)?;
    let budget = bounds::LiveBudget::new(transform_limits)?.with_retained(retained_bytes)?;
    check_cancel()?;
    let image = segmentation::load(&spec.source, analysis, resolve, &budget, check_cancel)?;
    finish(image, spec, output, resolve, &budget, check_cancel)
}

fn finish(
    mut image: RgbaImage,
    spec: &TransformSpec,
    output: &RasterOutputSpec,
    resolve: ResolveBytes<'_>,
    budget: &bounds::LiveBudget,
    check_cancel: CancelCheck<'_>,
) -> Result<RasterOutput, RasterError> {
    for step in &spec.steps {
        check_cancel()?;
        image = apply_step(image, step, resolve, budget, check_cancel)?;
    }
    let (actual_width_px, actual_height_px) = image.dimensions();
    let expected = output.dimensions();
    if actual_width_px != expected.width_px() || actual_height_px != expected.height_px() {
        return Err(RasterError::DimensionMismatch {
            actual_width_px,
            actual_height_px,
            expected_width_px: expected.width_px(),
            expected_height_px: expected.height_px(),
        });
    }
    let encoded = encode::encode_output(&image, output, budget, check_cancel)?;
    Ok(RasterOutput {
        width_px: actual_width_px,
        height_px: actual_height_px,
        has_alpha: encoded.has_alpha,
        mime: encoded.mime,
        bytes: encoded.bytes,
    })
}

fn load_source(
    source: &TransformSource,
    resolve: ResolveBytes<'_>,
    budget: &bounds::LiveBudget,
) -> Result<RgbaImage, RasterError> {
    match source {
        TransformSource::SegmentationMask { .. } => Err(RasterError::ResolveFailed),
        TransformSource::Image(pin) => {
            let bytes = decode::resolve_verified(resolve, pin, budget)?;
            decode::decode_bounded(&bytes, budget)
        }
        TransformSource::Coverage(coverage) => {
            let bytes = decode::resolve_verified(resolve, &coverage.coverage, budget)?;
            decode::decode_bounded(&bytes, budget)
        }
        TransformSource::Canvas(spec) => {
            budget.check_pixels(spec.width_px(), spec.height_px())?;
            let fill = spec.fill();
            Ok(RgbaImage::from_pixel(
                spec.width_px(),
                spec.height_px(),
                image::Rgba([fill.red, fill.green, fill.blue, 255]),
            ))
        }
    }
}

fn apply_step(
    image: RgbaImage,
    step: &ImageTransform,
    resolve: ResolveBytes<'_>,
    budget: &bounds::LiveBudget,
    check_cancel: CancelCheck<'_>,
) -> Result<RgbaImage, RasterError> {
    match step {
        ImageTransform::EncodedResize { dimensions, filter } => {
            encoded::resize(image, *dimensions, *filter, budget, check_cancel)
        }
        ImageTransform::EncodedThumbnail(dimensions) => {
            encoded::thumbnail(image, *dimensions, budget, check_cancel)
        }
        ImageTransform::EncodedMatte { fill, blend } => {
            encoded::matte(image, *fill, *blend, check_cancel)
        }
        ImageTransform::DiscardAlpha => encoded::discard_alpha(image, check_cancel),
        ImageTransform::Resize(spec) => geometry::apply_resize(image, spec, budget, check_cancel),
        ImageTransform::Crop(rect) => geometry::apply_crop(image, rect, budget, check_cancel),
        ImageTransform::Pad(spec) => geometry::apply_pad(image, spec, budget, check_cancel),
        ImageTransform::Rotate(spec) => geometry::apply_rotate(image, spec, budget, check_cancel),
        ImageTransform::Flip(axis) => geometry::apply_flip(image, *axis, budget, check_cancel),
        ImageTransform::Brighten(offset) => color::apply_brightness(image, *offset, check_cancel),
        ImageTransform::Expose(stops) => color::apply_exposure(image, *stops, check_cancel),
        ImageTransform::Color(effect) => color::apply_color_effect(image, *effect, check_cancel),
        ImageTransform::Blur(spec) => color::apply_blur(image, spec, budget, check_cancel),
        ImageTransform::Sharpen(spec) => color::apply_sharpen(image, spec, budget, check_cancel),
        ImageTransform::Mask(spec) => mask::apply_mask(image, spec, resolve, budget, check_cancel),
        ImageTransform::Composite(layers) => {
            mask::apply_composite(image, layers, resolve, budget, check_cancel)
        }
    }
}
