//! Provider-neutral historical image request and accounting records.
//! Native image execution belongs exclusively to df-imagery.

use crate::NativeHttpRefusal;
use std::fmt;

const MAXIMUM_REFERENCES: usize = 4;
const MAXIMUM_REFERENCE_BYTES: usize = 4 * 1024 * 1024;
const MINIMUM_PIXELS: u64 = 655_360;
const MAXIMUM_PIXELS: u64 = 4_194_304;

/// An edit model this adapter calls. Both accept `background: transparent` and any size whose
/// sides are multiples of 16.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImageEditModel {
    /// "Fast everyday generation".
    Gpt25Flare,
    /// "Editing precision".
    Gpt25Sunburst,
}

impl ImageEditModel {
    pub const fn model_id(self) -> &'static str {
        match self {
            Self::Gpt25Flare => "gpt-image-2.5-flare",
            Self::Gpt25Sunburst => "gpt-image-2.5-sunburst",
        }
    }

    /// The model whose id is exactly `id`; any other text is not a supported model.
    pub fn from_model_id(id: &str) -> Option<Self> {
        [Self::Gpt25Flare, Self::Gpt25Sunburst]
            .into_iter()
            .find(|model| model.model_id() == id)
    }
}

/// How hard the model works; it sets both cost and latency.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImageQuality {
    Low,
    Medium,
    High,
}

/// The container of an image: references are sent as they are stored, results come back in the
/// requested one (`Png`, `Jpeg` or `Webp`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImageFormat {
    Png,
    Jpeg,
    Webp,
}

/// Whether the result carries an alpha channel. Transparent results are always PNG.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImageBackground {
    Opaque,
    Transparent,
}

/// Supplier moderation strictness: `Low` is the less restrictive setting the API offers. It is
/// chosen once per request and never changed after a refusal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImageModeration {
    Auto,
    Low,
}

/// One reference image of an edit.
#[derive(Clone, Copy)]
pub struct ReferenceImage<'a> {
    pub format: ImageFormat,
    pub bytes: &'a [u8],
}

/// One edit: the references the model must keep faithful and the instruction to change them.
pub struct ImageEdit<'a> {
    pub model: ImageEditModel,
    pub prompt: &'a str,
    pub references: &'a [ReferenceImage<'a>],
    pub width: u32,
    pub height: u32,
    pub quality: ImageQuality,
    pub background: ImageBackground,
    /// `Png` or `Jpeg`; `Jpeg` is faster and cannot be transparent.
    pub output: ImageFormat,
    pub moderation: ImageModeration,
}

impl ImageEdit<'_> {
    /// Whether the edit passes the local checks every call makes before anything is sent, so a
    /// caller can charge a budget only for an edit that will be sent.
    pub fn validate(&self) -> Result<(), ImageEditError> {
        validate(self)
    }
}

/// Tokens the supplier billed for one call.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ImageUsage {
    pub text_input_tokens: u64,
    pub image_input_tokens: u64,
    pub output_tokens: u64,
}

impl ImageUsage {
    /// The published standard price in millionths of a US dollar: 5 per million text input
    /// tokens, 8 per million image input tokens and 30 per million output tokens.
    pub fn micro_dollars(self) -> u64 {
        self.text_input_tokens * 5 + self.image_input_tokens * 8 + self.output_tokens * 30
    }
}

/// A complete, structurally checked result.
#[derive(Clone, Eq, PartialEq)]
pub struct GeneratedImage {
    pub bytes: Vec<u8>,
    pub format: ImageFormat,
    pub width: u32,
    pub height: u32,
    /// The PNG colour type carries an alpha channel (grey+alpha or RGBA).
    pub has_alpha_channel: bool,
    pub usage: ImageUsage,
}

impl fmt::Debug for GeneratedImage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GeneratedImage")
            .field("byte_length", &self.bytes.len())
            .field("width", &self.width)
            .field("height", &self.height)
            .field("has_alpha_channel", &self.has_alpha_channel)
            .finish()
    }
}

/// Why a call produced no image. Never carries a prompt, an image or the supplier's text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImageEditError {
    /// The request failed local validation and nothing was sent.
    InvalidRequest,
    /// The supplier's moderation declined the prompt, a reference or the result. The caller
    /// keeps what it had; it must not reword or reroute.
    Refused,
    /// The supplier rejected the credential.
    Unauthorized,
    /// The supplier answered with another failure status, or rate limited the call (a 429
    /// without a quota reason). Waiting can clear it.
    Provider(u16),
    /// The supplier answered 429 with a structured reason that waiting does not clear: the
    /// account is out of credit or quota. Never retried.
    QuotaExhausted,
    Transport(NativeHttpRefusal),
    /// A success answer whose image was absent, undecodable, of another size or not complete.
    Output,
}

fn validate(edit: &ImageEdit<'_>) -> Result<(), ImageEditError> {
    let pixels = u64::from(edit.width) * u64::from(edit.height);
    let sides_ok = edit.width.is_multiple_of(16)
        && edit.height.is_multiple_of(16)
        && edit.width.max(edit.height) <= 3840
        && edit.width.max(edit.height) <= edit.width.min(edit.height) * 3;
    let references_ok = edit.references.len() <= MAXIMUM_REFERENCES
        && edit.references.iter().all(|reference| {
            !reference.bytes.is_empty() && reference.bytes.len() <= MAXIMUM_REFERENCE_BYTES
        });
    let output_ok = matches!(
        (edit.background, edit.output),
        (
            ImageBackground::Transparent,
            ImageFormat::Png | ImageFormat::Webp
        ) | (
            ImageBackground::Opaque,
            ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::Webp
        )
    );
    if edit.prompt.trim().is_empty()
        || edit.prompt.chars().count() > crate::IMAGE_EDIT_MAXIMUM_PROMPT_CHARACTERS
        || !references_ok
        || !sides_ok
        || !(MINIMUM_PIXELS..=MAXIMUM_PIXELS).contains(&pixels)
        || !output_ok
    {
        return Err(ImageEditError::InvalidRequest);
    }
    Ok(())
}
