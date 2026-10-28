//! Delivered raster contracts: generated size versus delivered variant.
//!
//! Provider-native generation size and delivered variant size are independent
//! facts; a local upscale never claims additional generated detail.

/// Pixel bound shared with the `df-assets` upload admission ceiling.
pub use df_assets::MAX_IMAGE_PIXELS as MAXIMUM_OUTPUT_PIXELS;
/// Neutral JPEG quality when the caller selects no explicit value.
pub const DEFAULT_JPEG_QUALITY_0_100: u8 = 85;

/// Shared pixel check: nonzero dimensions within the asset ceiling.
/// `u64` widening keeps the product overflow-free.
pub(crate) fn check_pixels(width_px: u32, height_px: u32) -> Result<(), OutputError> {
    if width_px == 0 || height_px == 0 {
        return Err(OutputError::ZeroDimension);
    }
    let pixels = u64::from(width_px) * u64::from(height_px);
    if pixels > MAXIMUM_OUTPUT_PIXELS {
        return Err(OutputError::TooManyPixels { pixels });
    }
    Ok(())
}

/// Initial output formats; each must be verified in the chosen encoder.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputFormat {
    Png,
    Jpeg,
    WebP,
}

impl OutputFormat {
    pub fn label(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpeg",
            Self::WebP => "webp",
        }
    }

    pub fn mime(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::WebP => "image/webp",
        }
    }

    pub fn supports_alpha(self) -> bool {
        !matches!(self, Self::Jpeg)
    }
}

/// Explicit pixel dimensions; construction refuses zero or over-bound sizes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Dimensions {
    width_px: u32,
    height_px: u32,
}

impl Dimensions {
    pub fn new(width_px: u32, height_px: u32) -> Result<Self, OutputError> {
        check_pixels(width_px, height_px)?;
        Ok(Self {
            width_px,
            height_px,
        })
    }

    pub fn width_px(self) -> u32 {
        self.width_px
    }

    pub fn height_px(self) -> u32 {
        self.height_px
    }

    pub fn pixel_count(self) -> u64 {
        u64::from(self.width_px) * u64::from(self.height_px)
    }
}

/// Opaque sRGB matte color for flattening onto formats without alpha.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Rgb {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

/// How alpha is handled at encode time. `Matte` names the explicit flatten
/// color used when the delivered pixels carry no alpha; it is meaningful on
/// every format, including alpha-capable ones flattened onto an opaque matte.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AlphaHandling {
    Opaque,
    PreserveAlpha,
    Matte(Rgb),
}

/// Lossy quality on a uniform 1..=100 scale; codecs map it explicitly.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LossyQuality(u8);

impl LossyQuality {
    pub fn new(quality_0_100: u8) -> Result<Self, OutputError> {
        if quality_0_100 == 0 || quality_0_100 > 100 {
            return Err(OutputError::QualityOutOfRange {
                value: quality_0_100,
            });
        }
        Ok(Self(quality_0_100))
    }

    pub fn value(self) -> u8 {
        self.0
    }
}

/// One final encode. Lossy and lossless are distinct; a JPEG quality number
/// is never reused as a PNG compression level.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Encoding {
    Jpeg { quality: LossyQuality },
    WebP { quality: Option<LossyQuality> },
    Png,
}

impl Encoding {
    pub fn format(self) -> OutputFormat {
        match self {
            Self::Jpeg { .. } => OutputFormat::Jpeg,
            Self::WebP { .. } => OutputFormat::WebP,
            Self::Png => OutputFormat::Png,
        }
    }

    pub fn is_lossless(self) -> bool {
        matches!(self, Self::Png | Self::WebP { quality: None })
    }
}

/// One delivered variant: explicit dimensions, alpha and encoding.
/// Construction validates; fields stay private so literals cannot bypass it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RasterOutputSpec {
    dimensions: Dimensions,
    alpha: AlphaHandling,
    encoding: Encoding,
}

impl RasterOutputSpec {
    pub fn new(
        dimensions: Dimensions,
        alpha: AlphaHandling,
        encoding: Encoding,
    ) -> Result<Self, OutputError> {
        let spec = Self {
            dimensions,
            alpha,
            encoding,
        };
        spec.validate()?;
        Ok(spec)
    }

    pub fn dimensions(self) -> Dimensions {
        self.dimensions
    }

    pub fn alpha(self) -> AlphaHandling {
        self.alpha
    }

    pub fn encoding(self) -> Encoding {
        self.encoding
    }

    /// Format-level check. Only JPEG with preserved alpha is refused here;
    /// a matte is an explicit flatten color and is allowed on every format.
    pub fn validate(&self) -> Result<(), OutputError> {
        if matches!(self.encoding.format(), OutputFormat::Jpeg)
            && matches!(self.alpha, AlphaHandling::PreserveAlpha)
        {
            return Err(OutputError::AlphaWithoutMatte);
        }
        Ok(())
    }

    /// Source-aware check the service runs after decoding: dropping alpha
    /// from alpha-bearing input requires an explicit matte. JPEG from opaque
    /// input needs no matte; the caller claim alone never proves opaqueness.
    pub fn validate_for_source_alpha(&self, source_has_alpha: bool) -> Result<(), OutputError> {
        self.validate()?;
        if source_has_alpha && matches!(self.alpha, AlphaHandling::Opaque) {
            return Err(OutputError::AlphaWithoutMatte);
        }
        Ok(())
    }
}

/// Delivered output authority: provider-backed generation versus local work.
/// `Provider` carries the provider-native generation size plus the delivered
/// variant; `Local` carries only the delivered variant, so a local canvas or
/// asset transform needs no fictitious generated size.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputProfile {
    Provider {
        generated: Dimensions,
        delivered: RasterOutputSpec,
    },
    Local {
        delivered: RasterOutputSpec,
    },
}

impl OutputProfile {
    pub fn provider(generated: Dimensions, delivered: RasterOutputSpec) -> Self {
        Self::Provider {
            generated,
            delivered,
        }
    }

    pub fn local(delivered: RasterOutputSpec) -> Self {
        Self::Local { delivered }
    }

    pub fn generated(self) -> Option<Dimensions> {
        match self {
            Self::Provider { generated, .. } => Some(generated),
            Self::Local { .. } => None,
        }
    }

    pub fn delivered(self) -> RasterOutputSpec {
        match self {
            Self::Provider { delivered, .. } | Self::Local { delivered } => delivered,
        }
    }
}

/// Why an output contract was refused before any work starts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputError {
    ZeroDimension,
    TooManyPixels { pixels: u64 },
    QualityOutOfRange { value: u8 },
    AlphaWithoutMatte,
}
