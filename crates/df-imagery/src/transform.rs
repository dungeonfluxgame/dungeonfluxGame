//! Ordered local raster recipes: geometry, color, mask, composite.
//!
//! Steps execute in listed order against the current oriented raster, so
//! crop-then-resize and resize-then-crop are different requests. Brightness
//! and exposure are separate operations with separate units and neutrals.
//! An empty step list is a valid encode-only transform.

use crate::output::{Dimensions, MAXIMUM_OUTPUT_PIXELS, Rgb};
use crate::request::{RequestLimits, SourceRef};

fn check_transform_pixels(width_px: u32, height_px: u32) -> Result<(), TransformError> {
    if width_px == 0 || height_px == 0 {
        return Err(TransformError::ZeroSize);
    }
    let pixels = u64::from(width_px) * u64::from(height_px);
    if pixels > MAXIMUM_OUTPUT_PIXELS {
        return Err(TransformError::TooManyPixels { pixels });
    }
    Ok(())
}

/// Source of a transform: a caller-claimed image pin, an explicitly sized
/// canvas, or coverage rasterized from bounded run data. Pins are claims
/// pending service authorization, never proof of access.
#[derive(Clone, Debug, PartialEq)]
pub enum TransformSource {
    Image(SourceRef),
    Canvas(CanvasSpec),
    Coverage(CoverageRef),
}

/// Explicitly sized and filled canvas; sheets need no fake file or upload.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CanvasSpec {
    width_px: u32,
    height_px: u32,
    fill: Rgb,
}

impl CanvasSpec {
    pub fn new(width_px: u32, height_px: u32, fill: Rgb) -> Result<Self, TransformError> {
        check_transform_pixels(width_px, height_px)?;
        Ok(Self {
            width_px,
            height_px,
            fill,
        })
    }

    pub fn width_px(self) -> u32 {
        self.width_px
    }

    pub fn height_px(self) -> u32 {
        self.height_px
    }

    pub fn fill(self) -> Rgb {
        self.fill
    }
}

/// Identifiable immutable coverage data bound to the analyzed source and the
/// analysis revision that produced it: the coverage bytes, what they were
/// derived from, and which derivation.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct CoverageRef {
    pub coverage: SourceRef,
    pub analyzed: SourceRef,
    pub analysis_revision: u32,
}

/// Integer pixel rectangle in the current oriented raster.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PixelRect {
    x_px: u32,
    y_px: u32,
    width_px: u32,
    height_px: u32,
}

impl PixelRect {
    pub fn new(
        x_px: u32,
        y_px: u32,
        width_px: u32,
        height_px: u32,
    ) -> Result<Self, TransformError> {
        check_transform_pixels(width_px, height_px)?;
        Ok(Self {
            x_px,
            y_px,
            width_px,
            height_px,
        })
    }

    pub fn x_px(self) -> u32 {
        self.x_px
    }

    pub fn y_px(self) -> u32 {
        self.y_px
    }

    pub fn width_px(self) -> u32 {
        self.width_px
    }

    pub fn height_px(self) -> u32 {
        self.height_px
    }
}

/// One anchor axis; explicit, never a magic coordinate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Anchor1D {
    Start,
    Center,
    End,
}

/// Crop and resize anchor with an explicit neutral default.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AlignAnchor {
    pub x: Anchor1D,
    pub y: Anchor1D,
}

impl AlignAnchor {
    pub fn center() -> Self {
        Self {
            x: Anchor1D::Center,
            y: Anchor1D::Center,
        }
    }
}

/// Resize semantics: `Cover` fills exact bounds via the anchor, `Pad` keeps
/// all content on an explicit background, `Stretch` changes aspect ratio.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResizeMode {
    Contain,
    Cover,
    Stretch,
    Pad,
}

/// Frozen named resampling set; no backend-dependent filter names.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Interpolation {
    Nearest,
    Bilinear,
    Bicubic,
    Lanczos3,
}

/// Target bounds plus how content reaches them; upscaling is opt-in. `Pad`
/// mode requires an explicit fill; other modes must not carry one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResizeSpec {
    width_px: u32,
    height_px: u32,
    mode: ResizeMode,
    anchor: AlignAnchor,
    interpolation: Interpolation,
    allow_upscale: bool,
    pad_fill: Option<Rgb>,
}

impl ResizeSpec {
    pub fn new(
        width_px: u32,
        height_px: u32,
        mode: ResizeMode,
        anchor: AlignAnchor,
        interpolation: Interpolation,
        allow_upscale: bool,
        pad_fill: Option<Rgb>,
    ) -> Result<Self, TransformError> {
        check_transform_pixels(width_px, height_px)?;
        match (mode, pad_fill) {
            (ResizeMode::Pad, None) => return Err(TransformError::MissingPadFill),
            (mode, Some(_)) if mode != ResizeMode::Pad => {
                return Err(TransformError::UnexpectedPadFill);
            }
            _ => {}
        }
        Ok(Self {
            width_px,
            height_px,
            mode,
            anchor,
            interpolation,
            allow_upscale,
            pad_fill,
        })
    }

    pub fn width_px(self) -> u32 {
        self.width_px
    }

    pub fn height_px(self) -> u32 {
        self.height_px
    }

    pub fn mode(self) -> ResizeMode {
        self.mode
    }

    pub fn anchor(self) -> AlignAnchor {
        self.anchor
    }

    pub fn interpolation(self) -> Interpolation {
        self.interpolation
    }

    pub fn allow_upscale(self) -> bool {
        self.allow_upscale
    }

    pub fn pad_fill(self) -> Option<Rgb> {
        self.pad_fill
    }
}

/// Exact right angles first; arbitrary angles name canvas and fill behavior
/// plus the interpolation used for resampling.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RotateSpec {
    QuarterTurns(u8),
    Arbitrary {
        degrees: f32,
        expand_canvas: bool,
        fill: Rgb,
        interpolation: Interpolation,
    },
}

impl RotateSpec {
    pub(crate) fn validate(&self) -> Result<(), TransformError> {
        match self {
            Self::QuarterTurns(turns) => {
                if *turns > 3 {
                    return Err(TransformError::OutOfRange { field: "rotation" });
                }
                Ok(())
            }
            Self::Arbitrary { degrees, .. } => {
                if !degrees.is_finite() {
                    return Err(TransformError::NonFinite { field: "rotation" });
                }
                Ok(())
            }
        }
    }
}

/// Mirror axis.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FlipAxis {
    Horizontal,
    Vertical,
}

/// Signed normalized offset in the declared working RGB space; neutral zero.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BrightnessOffset(f32);

/// Stops of exposure in linear light (`2^stops`); neutral zero, alpha kept.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExposureStops(f32);

impl BrightnessOffset {
    pub fn new(offset: f32) -> Result<Self, TransformError> {
        if !offset.is_finite() {
            return Err(TransformError::NonFinite {
                field: "brightness",
            });
        }
        Ok(Self(offset))
    }

    pub fn is_neutral(self) -> bool {
        self.0 == 0.0
    }

    pub fn value(self) -> f32 {
        self.0
    }
}

impl ExposureStops {
    pub fn new(stops: f32) -> Result<Self, TransformError> {
        if !stops.is_finite() {
            return Err(TransformError::NonFinite { field: "exposure" });
        }
        Ok(Self(stops))
    }

    pub fn is_neutral(self) -> bool {
        self.0 == 0.0
    }

    pub fn value(self) -> f32 {
        self.0
    }
}

/// One color adjustment; alpha is unchanged unless the operation targets it.
/// Bounds below are mathematical: finite inputs, positive gain, a normalized
/// pivot, and nonnegative saturation. No arbitrary magnitude caps.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ColorEffect {
    Contrast { gain: f32, pivot: f32 },
    Saturation { factor: f32 },
    HueShift { degrees: f32 },
    Grayscale,
}

impl ColorEffect {
    pub fn contrast(gain: f32, pivot: f32) -> Result<Self, TransformError> {
        if !gain.is_finite() || !pivot.is_finite() {
            return Err(TransformError::NonFinite { field: "contrast" });
        }
        if gain <= 0.0 || !(0.0..=1.0).contains(&pivot) {
            return Err(TransformError::OutOfRange { field: "contrast" });
        }
        Ok(Self::Contrast { gain, pivot })
    }

    pub fn saturation(factor: f32) -> Result<Self, TransformError> {
        if !factor.is_finite() {
            return Err(TransformError::NonFinite {
                field: "saturation",
            });
        }
        if factor < 0.0 {
            return Err(TransformError::OutOfRange {
                field: "saturation",
            });
        }
        Ok(Self::Saturation { factor })
    }

    pub fn hue_shift(degrees: f32) -> Result<Self, TransformError> {
        if !degrees.is_finite() {
            return Err(TransformError::NonFinite { field: "hue" });
        }
        Ok(Self::HueShift { degrees })
    }

    pub(crate) fn validate(&self) -> Result<(), TransformError> {
        match *self {
            Self::Contrast { gain, pivot } => Self::contrast(gain, pivot).map(|_| ()),
            Self::Saturation { factor } => Self::saturation(factor).map(|_| ()),
            Self::HueShift { degrees } => Self::hue_shift(degrees).map(|_| ()),
            Self::Grayscale => Ok(()),
        }
    }
}

/// Named blur kernel; radius is in current-raster pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BlurAlgorithm {
    Box,
    Gaussian,
}

/// Blur radius in current-raster pixels with its named kernel. Any positive
/// finite radius is representable; resource bounds belong to configuration.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlurSpec {
    radius_px: f32,
    algorithm: BlurAlgorithm,
}

impl BlurSpec {
    pub fn new(radius_px: f32, algorithm: BlurAlgorithm) -> Result<Self, TransformError> {
        if !radius_px.is_finite() {
            return Err(TransformError::NonFinite { field: "blur" });
        }
        if radius_px <= 0.0 {
            return Err(TransformError::OutOfRange { field: "blur" });
        }
        Ok(Self {
            radius_px,
            algorithm,
        })
    }

    pub fn radius_px(self) -> f32 {
        self.radius_px
    }

    pub fn algorithm(self) -> BlurAlgorithm {
        self.algorithm
    }
}

/// Sharpen radius, strength factor and noise threshold.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SharpenSpec {
    radius_px: f32,
    amount_factor: f32,
    threshold_0_255: u8,
}

impl SharpenSpec {
    pub fn new(
        radius_px: f32,
        amount_factor: f32,
        threshold_0_255: u8,
    ) -> Result<Self, TransformError> {
        if !radius_px.is_finite() || !amount_factor.is_finite() {
            return Err(TransformError::NonFinite { field: "sharpen" });
        }
        if radius_px <= 0.0 || amount_factor < 0.0 {
            return Err(TransformError::OutOfRange { field: "sharpen" });
        }
        Ok(Self {
            radius_px,
            amount_factor,
            threshold_0_255,
        })
    }

    pub fn radius_px(self) -> f32 {
        self.radius_px
    }

    pub fn amount_factor(self) -> f32 {
        self.amount_factor
    }

    pub fn threshold_0_255(self) -> u8 {
        self.threshold_0_255
    }
}

/// Which channel feeds the mask; semantic segmentation is never implied.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MaskChannel {
    Alpha,
    Luminance,
}

/// Multiply keeps content; Replace writes coverage as alpha.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MaskMode {
    Multiply,
    Replace,
}

/// Where mask coverage comes from: a claimed immutable asset or claimed
/// coverage data bound to its analyzed source. Claims pending authorization.
#[derive(Clone, Debug, PartialEq)]
pub enum MaskSource {
    Asset(SourceRef),
    Coverage(CoverageRef),
}

/// Scalar coverage application with explicit alignment, sizing and channel
/// policy; never semantic segmentation.
#[derive(Clone, Debug, PartialEq)]
pub struct MaskSpec {
    source: MaskSource,
    dimensions: Dimensions,
    align: AlignAnchor,
    channel: MaskChannel,
    invert: bool,
    feather_radius_px: f32,
    mode: MaskMode,
}

impl MaskSpec {
    pub fn new(
        source: MaskSource,
        dimensions: Dimensions,
        align: AlignAnchor,
        channel: MaskChannel,
        invert: bool,
        feather_radius_px: f32,
        mode: MaskMode,
    ) -> Result<Self, TransformError> {
        if !feather_radius_px.is_finite() {
            return Err(TransformError::NonFinite { field: "mask" });
        }
        if feather_radius_px < 0.0 {
            return Err(TransformError::OutOfRange { field: "mask" });
        }
        Ok(Self {
            source,
            dimensions,
            align,
            channel,
            invert,
            feather_radius_px,
            mode,
        })
    }

    pub fn source(&self) -> &MaskSource {
        &self.source
    }

    pub fn dimensions(&self) -> Dimensions {
        self.dimensions
    }

    pub fn width_px(&self) -> u32 {
        self.dimensions.width_px()
    }

    pub fn height_px(&self) -> u32 {
        self.dimensions.height_px()
    }

    pub fn align(&self) -> AlignAnchor {
        self.align
    }

    pub fn channel(&self) -> MaskChannel {
        self.channel
    }

    pub fn invert(&self) -> bool {
        self.invert
    }

    pub fn feather_radius_px(&self) -> f32 {
        self.feather_radius_px
    }

    pub fn mode(&self) -> MaskMode {
        self.mode
    }
}

/// Only source-over until another mode ships defined equations plus fixtures.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlendMode {
    SourceOver,
}

/// One ordered layer over the current raster; opacity is permille. The layer
/// extent is the layer asset's decoded extent applied at the position.
#[derive(Clone, Debug, PartialEq)]
pub struct CompositeLayer {
    source: SourceRef,
    x_px: i32,
    y_px: i32,
    opacity_permille: u16,
    blend: BlendMode,
}

impl CompositeLayer {
    pub fn new(
        source: SourceRef,
        x_px: i32,
        y_px: i32,
        opacity_permille: u16,
        blend: BlendMode,
    ) -> Result<Self, TransformError> {
        if opacity_permille > 1000 {
            return Err(TransformError::OutOfRange { field: "opacity" });
        }
        Ok(Self {
            source,
            x_px,
            y_px,
            opacity_permille,
            blend,
        })
    }

    pub fn source(&self) -> &SourceRef {
        &self.source
    }

    pub fn x_px(&self) -> i32 {
        self.x_px
    }

    pub fn y_px(&self) -> i32 {
        self.y_px
    }

    pub fn opacity_permille(&self) -> u16 {
        self.opacity_permille
    }

    pub fn blend(&self) -> BlendMode {
        self.blend
    }
}

/// Padding to explicit canvas dimensions on an explicit background color.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PadSpec {
    width_px: u32,
    height_px: u32,
    anchor: AlignAnchor,
    fill: Rgb,
}

impl PadSpec {
    pub fn new(
        width_px: u32,
        height_px: u32,
        anchor: AlignAnchor,
        fill: Rgb,
    ) -> Result<Self, TransformError> {
        check_transform_pixels(width_px, height_px)?;
        Ok(Self {
            width_px,
            height_px,
            anchor,
            fill,
        })
    }

    pub fn width_px(self) -> u32 {
        self.width_px
    }

    pub fn height_px(self) -> u32 {
        self.height_px
    }

    pub fn anchor(self) -> AlignAnchor {
        self.anchor
    }

    pub fn fill(self) -> Rgb {
        self.fill
    }
}

/// One ordered step of a local recipe.
#[derive(Clone, Debug, PartialEq)]
pub enum ImageTransform {
    Resize(ResizeSpec),
    Crop(PixelRect),
    Pad(PadSpec),
    Rotate(RotateSpec),
    Flip(FlipAxis),
    Brighten(BrightnessOffset),
    Expose(ExposureStops),
    Color(ColorEffect),
    Blur(BlurSpec),
    Sharpen(SharpenSpec),
    Mask(MaskSpec),
    Composite(Vec<CompositeLayer>),
}

/// Ordered local recipe: source plus ordered local steps. Delivery is owned
/// by the outer request output; an empty step list is a valid no-op recipe.
#[derive(Clone, Debug, PartialEq)]
pub struct TransformSpec {
    pub source: TransformSource,
    pub steps: Vec<ImageTransform>,
}

impl TransformSpec {
    /// Boundary validation against injected limits. Private-field structs
    /// already enforce their invariants at construction; only openly
    /// constructible enum payloads and injected counts are rechecked here.
    pub fn validate(&self, limits: &RequestLimits) -> Result<(), TransformError> {
        if self.steps.len() > limits.max_transform_steps {
            return Err(TransformError::TooManySteps);
        }
        for step in &self.steps {
            match step {
                ImageTransform::Rotate(spec) => spec.validate()?,
                ImageTransform::Color(effect) => effect.validate()?,
                ImageTransform::Composite(layers) => {
                    if layers.is_empty() {
                        return Err(TransformError::EmptyLayers);
                    }
                    if layers.len() > limits.max_composite_layers {
                        return Err(TransformError::TooManyLayers);
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
}

/// Why a transform recipe was refused before any pixel work.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransformError {
    TooManySteps,
    EmptyLayers,
    TooManyLayers,
    ZeroSize,
    TooManyPixels { pixels: u64 },
    NonFinite { field: &'static str },
    OutOfRange { field: &'static str },
    MissingPadFill,
    UnexpectedPadFill,
}
