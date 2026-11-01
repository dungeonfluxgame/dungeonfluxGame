//! Proof of decoded image bytes, bound to one immutable written catalog variant.

use crate::{AssetVariant, VariantRecord, VariantState};
use image::{ImageFormat, ImageReader, Limits};
use sha2::{Digest, Sha256};
use std::io::Cursor;

/// Runtime-injected publication bounds; generation does not inherit upload resource policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ImageProofLimits {
    maximum_encoded_bytes: u64,
    maximum_pixels: u64,
    maximum_decoded_bytes: u64,
}

impl ImageProofLimits {
    pub fn new(
        maximum_encoded_bytes: u64,
        maximum_pixels: u64,
        maximum_decoded_bytes: u64,
    ) -> Result<Self, ImageProofError> {
        if maximum_encoded_bytes == 0 || maximum_pixels == 0 || maximum_decoded_bytes == 0 {
            return Err(ImageProofError::InvalidLimits);
        }
        Ok(Self {
            maximum_encoded_bytes,
            maximum_pixels,
            maximum_decoded_bytes,
        })
    }
    pub fn check_encoded_len(self, length: u64) -> Result<(), ImageProofError> {
        if length > self.maximum_encoded_bytes {
            return Err(ImageProofError::TooLarge);
        }
        Ok(())
    }
    pub fn check_dimensions(self, width: u32, height: u32) -> Result<(), ImageProofError> {
        if width == 0 || height == 0 || u64::from(width) * u64::from(height) > self.maximum_pixels {
            return Err(ImageProofError::TooManyPixels);
        }
        Ok(())
    }
}

impl Default for ImageProofLimits {
    fn default() -> Self {
        Self {
            maximum_encoded_bytes: 64 * 1024 * 1024,
            maximum_pixels: 20_000_000,
            maximum_decoded_bytes: 160_000_000,
        }
    }
}

/// Whether any decoded pixel is less than fully opaque.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImageAlpha {
    Opaque,
    Transparent,
}

/// Frozen alpha policy is distinct from the actual alpha measured in decoded pixels.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImageAlphaRequirement {
    Preserve,
    Opaque,
    Transparent,
}
impl From<ImageAlpha> for ImageAlphaRequirement {
    fn from(alpha: ImageAlpha) -> Self {
        match alpha {
            ImageAlpha::Opaque => Self::Opaque,
            ImageAlpha::Transparent => Self::Transparent,
        }
    }
}
impl ImageAlphaRequirement {
    pub fn accepts(self, alpha: ImageAlpha) -> bool {
        match self {
            Self::Preserve => true,
            Self::Opaque => alpha == ImageAlpha::Opaque,
            Self::Transparent => alpha == ImageAlpha::Transparent,
        }
    }
}

/// Exact output properties required by the frozen image recipe.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ImageIntent {
    mime: &'static str,
    width_px: u32,
    height_px: u32,
    alpha: ImageAlphaRequirement,
}

impl ImageIntent {
    pub fn new(
        mime: &str,
        width_px: u32,
        height_px: u32,
        alpha: impl Into<ImageAlphaRequirement>,
    ) -> Result<Self, ImageProofError> {
        let mime = match mime {
            "image/png" => "image/png",
            "image/jpeg" => "image/jpeg",
            "image/webp" => "image/webp",
            _ => return Err(ImageProofError::UnsupportedFormat),
        };
        if width_px == 0
            || height_px == 0
            || i32::try_from(width_px).is_err()
            || i32::try_from(height_px).is_err()
        {
            return Err(ImageProofError::TooManyPixels);
        }
        Ok(Self {
            mime,
            width_px,
            height_px,
            alpha: alpha.into(),
        })
    }

    pub fn mime(self) -> &'static str {
        self.mime
    }
    pub fn width_px(self) -> u32 {
        self.width_px
    }
    pub fn height_px(self) -> u32 {
        self.height_px
    }
    pub fn alpha_requirement(self) -> ImageAlphaRequirement {
        self.alpha
    }
    pub fn accepts(self, actual: Self, alpha: ImageAlpha) -> bool {
        self.mime == actual.mime
            && self.width_px == actual.width_px
            && self.height_px == actual.height_px
            && self.alpha.accepts(alpha)
    }
}

/// Only `decode` can construct this proof; caller metadata cannot attest byte properties.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImageProof {
    record: VariantRecord,
    recipe_digest: [u8; 32],
    intent: ImageIntent,
    alpha: ImageAlpha,
}

/// A typed rejection before any publication facts are emitted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImageProofError {
    InvalidLimits,
    NotWritten,
    WrongDomain,
    TooLarge,
    IntegrityMismatch,
    UnsupportedFormat,
    InvalidImage,
    TooManyPixels,
    IntentMismatch,
}

impl ImageProof {
    /// Decodes exact durable bytes with bounded allocation and compares the immutable manifest.
    /// Run this CPU work on the caller's bounded blocking worker before opening a transaction.
    pub fn decode(
        record: &VariantRecord,
        bytes: &[u8],
        recipe_digest: [u8; 32],
        intent: ImageIntent,
        bounds: ImageProofLimits,
    ) -> Result<Self, ImageProofError> {
        if record.state != VariantState::Written || record.version.is_none() {
            return Err(ImageProofError::NotWritten);
        }
        if record.key.domain() != crate::StorageDomain::PrivateMedia {
            return Err(ImageProofError::WrongDomain);
        }
        bounds.check_encoded_len(
            u64::try_from(bytes.len()).map_err(|_| ImageProofError::TooLarge)?,
        )?;
        let manifest = record.manifest.ok_or(ImageProofError::NotWritten)?;
        let length = u64::try_from(bytes.len()).map_err(|_| ImageProofError::TooLarge)?;
        let digest: [u8; 32] = Sha256::digest(bytes).into();
        if manifest.byte_len != length || manifest.sha256 != digest {
            return Err(ImageProofError::IntegrityMismatch);
        }
        let alpha = Self::inspect_encoded(bytes, intent, bounds)?;
        let actual = ImageIntent::new(intent.mime(), intent.width_px(), intent.height_px(), alpha)?;
        Ok(Self {
            record: record.clone(),
            recipe_digest,
            intent: actual,
            alpha,
        })
    }

    /// Validates staged encoded pixels without emitting a catalog-bound publication proof.
    pub fn inspect_encoded(
        bytes: &[u8],
        intent: ImageIntent,
        bounds: ImageProofLimits,
    ) -> Result<ImageAlpha, ImageProofError> {
        bounds.check_encoded_len(
            u64::try_from(bytes.len()).map_err(|_| ImageProofError::TooLarge)?,
        )?;
        let format = image::guess_format(bytes).map_err(|_| ImageProofError::UnsupportedFormat)?;
        let mime = match format {
            ImageFormat::Png => "image/png",
            ImageFormat::Jpeg => "image/jpeg",
            ImageFormat::WebP => "image/webp",
            _ => return Err(ImageProofError::UnsupportedFormat),
        };
        if crate::validation::is_animated(bytes, format)
            .map_err(|_| ImageProofError::InvalidImage)?
        {
            return Err(ImageProofError::InvalidImage);
        }
        let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
        let mut limits = Limits::default();
        limits.max_image_width = Some(intent.width_px());
        limits.max_image_height = Some(intent.height_px());
        limits.max_alloc = Some(bounds.maximum_decoded_bytes);
        reader.limits(limits);
        let (width, height) = ImageReader::with_format(Cursor::new(bytes), format)
            .into_dimensions()
            .map_err(|_| ImageProofError::InvalidImage)?;
        bounds.check_dimensions(width, height)?;
        if width != intent.width_px() || height != intent.height_px() || mime != intent.mime() {
            return Err(ImageProofError::IntentMismatch);
        }
        let decoded = reader.decode().map_err(|_| ImageProofError::InvalidImage)?;
        let alpha = measured_alpha(&decoded)?;
        let actual = ImageIntent::new(mime, decoded.width(), decoded.height(), alpha)?;
        if !intent.accepts(actual, alpha) {
            return Err(ImageProofError::IntentMismatch);
        }
        Ok(alpha)
    }

    pub fn record(&self) -> &VariantRecord {
        &self.record
    }
    pub fn recipe_digest(&self) -> &[u8; 32] {
        &self.recipe_digest
    }
    pub fn intent(&self) -> ImageIntent {
        self.intent
    }
    pub fn alpha(&self) -> ImageAlpha {
        self.alpha
    }
    pub fn variant(&self) -> AssetVariant {
        self.record.variant
    }
}

fn measured_alpha(image: &image::DynamicImage) -> Result<ImageAlpha, ImageProofError> {
    use image::DynamicImage;
    let transparent = match image {
        DynamicImage::ImageLumaA8(pixels) => pixels.pixels().any(|pixel| pixel.0[1] < u8::MAX),
        DynamicImage::ImageRgba8(pixels) => pixels.pixels().any(|pixel| pixel.0[3] < u8::MAX),
        DynamicImage::ImageLumaA16(pixels) => pixels.pixels().any(|pixel| pixel.0[1] < u16::MAX),
        DynamicImage::ImageRgba16(pixels) => pixels.pixels().any(|pixel| pixel.0[3] < u16::MAX),
        other if !other.color().has_alpha() => false,
        _ => return Err(ImageProofError::InvalidImage),
    };
    Ok(if transparent {
        ImageAlpha::Transparent
    } else {
        ImageAlpha::Opaque
    })
}
