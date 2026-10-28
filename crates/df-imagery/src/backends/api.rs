//! Private API image backends: OpenAI, fal and BFL execution.
//!
//! This module owns the relocated supplier execution previously in
//! `df-providers::image_edit`, `df-providers::fal_image` and
//! `df-providers::bfl_image`. Old files stay untouched until the coordinator
//! migrates callers and removes the bypass. Nothing here is public outside
//! `df-imagery`: the later service exposes behavior, never these executors.
//!
//! What this module never does: retry, reword after refusal, reroute to
//! another supplier, log prompts or bytes, or record spend. One fal edit with
//! a transparent background is two separately metered stages; the caller sees
//! both receipts and never a single merged charge.

pub(crate) mod bfl;
pub(crate) mod codec;
pub(crate) mod fal;
pub(crate) mod openai;

#[cfg(test)]
mod bfl_tests;
#[cfg(test)]
mod codec_tests;
#[cfg(test)]
mod fal_tests;
#[cfg(test)]
mod openai_tests;

use std::time::Duration;

pub(crate) use bfl::{BflConfig, BflExecutor, BflFailure, BflSuccess};
pub(crate) use fal::{FalAlphaReceipt, FalConfig, FalEditOutcome, FalExecutor, FalGenerationReceipt};
pub(crate) use openai::{OpenAiConfig, OpenAiExecutor};

/// Reused supplier prompt ceiling; kept as a single choke point.
pub(crate) use df_providers::IMAGE_EDIT_MAXIMUM_PROMPT_CHARACTERS;
/// Shared transport refusal; callers branch on variants, never on text.
pub(crate) use df_providers::NativeHttpRefusal;

/// Why a private API stage produced no image. Never carries a prompt, bytes
/// or supplier text; refusal and quota outcomes are terminal by contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ApiError {
    InvalidRequest,
    Refused,
    Unauthorized,
    Provider(u16),
    QuotaExhausted,
    Transport(NativeHttpRefusal),
    Output,
}

/// An edit model this backend calls. Both accept transparent output.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ImageEditModel {
    Gpt25Flare,
    Gpt25Sunburst,
}

impl ImageEditModel {
    pub(crate) const fn model_id(self) -> &'static str {
        match self {
            Self::Gpt25Flare => "gpt-image-2.5-flare",
            Self::Gpt25Sunburst => "gpt-image-2.5-sunburst",
        }
    }

    pub(crate) fn from_model_id(id: &str) -> Option<Self> {
        [Self::Gpt25Flare, Self::Gpt25Sunburst]
            .into_iter()
            .find(|model| model.model_id() == id)
    }
}

/// How hard the model works; it sets both cost and latency.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ImageQuality {
    Low,
    Medium,
    High,
}

impl ImageQuality {
    pub(crate) const fn wire(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

/// Container of an image: references arrive as stored, results leave in one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ImageFormat {
    Png,
    Jpeg,
    Webp,
}

impl ImageFormat {
    pub(crate) const fn mime(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Webp => "image/webp",
        }
    }

    pub(crate) const fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::Webp => "webp",
        }
    }

    pub(crate) const fn wire(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpeg",
            Self::Webp => "webp",
        }
    }
}

/// Whether the result carries an alpha channel. Transparent is always PNG.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ImageBackground {
    Opaque,
    Transparent,
}

/// Supplier moderation strictness, chosen once per request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ImageModeration {
    Auto,
    Low,
}

/// One reference image of an edit.
#[derive(Clone, Copy)]
pub(crate) struct ReferenceImage<'a> {
    pub(crate) format: ImageFormat,
    pub(crate) bytes: &'a [u8],
}

/// One edit: faithful references plus the instruction changing them.
pub(crate) struct ImageEdit<'a> {
    pub(crate) model: ImageEditModel,
    pub(crate) prompt: &'a str,
    pub(crate) references: &'a [ReferenceImage<'a>],
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) quality: ImageQuality,
    pub(crate) background: ImageBackground,
    pub(crate) output: ImageFormat,
    pub(crate) moderation: ImageModeration,
}

/// Tokens the supplier billed for one OpenAI call.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ImageUsage {
    pub(crate) text_input_tokens: u64,
    pub(crate) image_input_tokens: u64,
    pub(crate) output_tokens: u64,
}

impl ImageUsage {
    pub(crate) fn micro_dollars(self) -> u64 {
        self.text_input_tokens * 5 + self.image_input_tokens * 8 + self.output_tokens * 30
    }
}

/// A complete, structurally checked result with actual bytes and usage.
#[derive(Clone, Eq, PartialEq)]
pub(crate) struct GeneratedImage {
    pub(crate) bytes: Vec<u8>,
    pub(crate) format: ImageFormat,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) has_alpha_channel: bool,
    pub(crate) usage: ImageUsage,
}

impl std::fmt::Debug for GeneratedImage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GeneratedImage")
            .field("byte_length", &self.bytes.len())
            .field("width", &self.width)
            .field("height", &self.height)
            .field("has_alpha_channel", &self.has_alpha_channel)
            .finish()
    }
}

/// Injectable operational bounds. Defaults preserve current compatibility
/// behavior; composition supplies the snapshot, never ambient state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct OperationalBounds {
    pub(crate) openai_max_request_bytes: usize,
    pub(crate) openai_max_response_bytes: usize,
    pub(crate) fal_max_request_bytes: usize,
    pub(crate) fal_max_response_bytes: usize,
    pub(crate) fal_max_image_bytes: usize,
    pub(crate) bfl_max_delivery_bytes: usize,
    pub(crate) poll_interval: Duration,
}

impl OperationalBounds {
    pub(crate) fn compatibility_defaults() -> Self {
        Self {
            openai_max_request_bytes: 12 * 1024 * 1024,
            openai_max_response_bytes: 16 * 1024 * 1024,
            fal_max_request_bytes: 16 * 1024 * 1024,
            fal_max_response_bytes: 16 * 1024 * 1024,
            fal_max_image_bytes: 11 * 1024 * 1024,
            bfl_max_delivery_bytes: 6 * 1024 * 1024,
            poll_interval: Duration::from_millis(500),
        }
    }

    pub(crate) fn validate(self) -> Result<(), ApiError> {
        if self.openai_max_request_bytes == 0
            || self.openai_max_response_bytes == 0
            || self.fal_max_request_bytes == 0
            || self.fal_max_response_bytes == 0
            || self.fal_max_image_bytes == 0
            || self.bfl_max_delivery_bytes == 0
            || self.poll_interval.is_zero()
        {
            return Err(ApiError::InvalidRequest);
        }
        Ok(())
    }
}

/// Shared credential gate: printable ASCII, 1..=4096 bytes. Secrets never
/// reach diagnostics; only the refusal outcome survives.
pub(crate) fn check_credential(credential: &str) -> Result<(), ApiError> {
    if credential.is_empty()
        || credential.len() > 4096
        || !credential.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
    {
        return Err(ApiError::InvalidRequest);
    }
    Ok(())
}
