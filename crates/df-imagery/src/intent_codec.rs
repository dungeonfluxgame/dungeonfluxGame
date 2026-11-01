//! Private canonical binary consumer-intent and config snapshot codec.
//!
//! Encodes the consumer intent inside one [`ImageRequest`] without the caller
//! identity that must never steer bytes: the operation id, the caller
//! fingerprint and the scheduling urgency stay out, as do resolved routing and
//! classification. A second encoding covers the full [`ImageConfig`] snapshot.
//! Both encodings use explicit versions, one-byte tags, big-endian integer
//! widths and `u32` length prefixes, keep member sets in canonical order,
//! canonicalize floats and never serialize `Debug` output. Decoding is bounded
//! and rebuilds values through the validated constructors so a restart
//! replays checked work.

mod artifact_semantics;
mod artwork;
#[cfg(test)]
mod artwork_tests;
pub(crate) mod decode;
pub(crate) mod encode;
pub(crate) mod pipeline;
#[cfg(test)]
mod pipeline_tests;
pub(crate) mod primitives;

pub(crate) use artifact_semantics::encode as encode_artifact_semantics;
pub(crate) use decode::{decode_config, decode_intent};
pub(crate) use encode::{encode_config, encode_intent};

use df_assets::Audience;
use std::fmt;

use crate::config::ConfigError;
use crate::output::{OutputError, OutputProfile};
use crate::request::{
    ExecutionMode, Freshness, ImageOperation, ImageOwner, PurposedUse, RequestError,
};

/// Legacy intent encoding stays byte-exact for requests without pipeline metadata.
pub(crate) const INTENT_CODEC_VERSION: u8 = 1;
pub(super) const LEGACY_PIPELINE_INTENT_CODEC_VERSION: u8 = 2;
pub(super) const INSPECTION_REQUIREMENT_CODEC_VERSION: u8 = 3;
pub(super) const MODERATION_INTENT_CODEC_VERSION: u8 = 4;
pub(super) const PIPELINE_INTENT_CODEC_VERSION: u8 = 5;
pub(super) const ARTWORK_INTENT_CODEC_VERSION: u8 = 6;

pub(crate) fn intent_version(bytes: &[u8]) -> Result<u8, CodecError> {
    match bytes.first().copied() {
        Some(
            version @ (INTENT_CODEC_VERSION
            | LEGACY_PIPELINE_INTENT_CODEC_VERSION
            | INSPECTION_REQUIREMENT_CODEC_VERSION
            | MODERATION_INTENT_CODEC_VERSION
            | PIPELINE_INTENT_CODEC_VERSION
            | ARTWORK_INTENT_CODEC_VERSION),
        ) => Ok(version),
        Some(actual) => Err(CodecError::BadVersion { actual }),
        None => Err(CodecError::Truncated),
    }
}
/// Version 12 appends the explicit creator BFL account after the frozen v11 quota policy.
/// Every earlier version retains its original field defaults and exact layout.
pub(crate) const CONFIG_CODEC_VERSION: u8 = 12;

/// Why an intent or config snapshot was refused before any work starts. The
/// variants carry lengths and tags only; prompt text never appears here.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CodecError {
    Request(RequestError),
    Output(OutputError),
    Config(ConfigError),
    BadVersion { actual: u8 },
    Truncated,
    Trailing { extra_bytes: usize },
    TooLong,
    Malformed,
    InvalidText,
    InvalidId,
    NonFinite { field: &'static str },
}

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Request(error) => write!(f, "intent rejected: {error:?}"),
            Self::Output(error) => write!(f, "output rejected: {error:?}"),
            Self::Config(error) => write!(f, "config rejected: {error:?}"),
            Self::BadVersion { actual } => write!(f, "unsupported codec version {actual}"),
            Self::Truncated => write!(f, "bytes end before the value does"),
            Self::Trailing { extra_bytes } => {
                write!(f, "{extra_bytes} trailing bytes after the value")
            }
            Self::TooLong => write!(f, "value does not fit the byte bound"),
            Self::Malformed => write!(f, "bytes do not form a known shape"),
            Self::InvalidText => write!(f, "bytes are not valid text"),
            Self::InvalidId => write!(f, "bytes are not a valid identity"),
            Self::NonFinite { field } => write!(f, "{field} is not finite"),
        }
    }
}

impl std::error::Error for CodecError {}

/// Encoded consumer intent: version, canonical bytes and their SHA-256.
/// `Debug` prints lengths and the fingerprint, never prompt text or raw bytes.
pub(crate) struct EncodedIntent {
    version: u8,
    bytes: Vec<u8>,
    fingerprint: [u8; 32],
}

impl EncodedIntent {
    #[cfg(test)]
    pub(crate) fn version(&self) -> u8 {
        self.version
    }

    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub(crate) fn fingerprint(&self) -> &[u8; 32] {
        &self.fingerprint
    }
}

impl fmt::Debug for EncodedIntent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EncodedIntent")
            .field("version", &self.version)
            .field("len_bytes", &self.bytes.len())
            .field("fingerprint", &hex(&self.fingerprint))
            .finish()
    }
}

/// Encoded config snapshot. `Debug` prints lengths only, never credentials.
pub(crate) struct EncodedConfig {
    version: u8,
    bytes: Vec<u8>,
    fingerprint: [u8; 32],
}

impl EncodedConfig {
    pub(crate) fn version(&self) -> u8 {
        self.version
    }

    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    #[cfg(test)]
    pub(crate) fn fingerprint(&self) -> &[u8; 32] {
        &self.fingerprint
    }
}

impl fmt::Debug for EncodedConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EncodedConfig")
            .field("version", &self.version)
            .field("len_bytes", &self.bytes.len())
            .field("fingerprint", &hex(&self.fingerprint))
            .finish()
    }
}

/// Decoded consumer intent for restart reconstruction. It carries no operation
/// id, caller fingerprint or scheduling urgency: those never steer bytes.
/// `Debug` names shapes and lengths only, never prompt text.
pub(crate) struct DecodedIntent {
    pub owner: ImageOwner,
    pub purpose: PurposedUse,
    pub operation: ImageOperation,
    pub execution: ExecutionMode,
    pub freshness: Freshness,
    pub audience: Audience,
    pub expected_source_revision: u64,
    pub output: OutputProfile,
}

impl fmt::Debug for DecodedIntent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.operation {
            ImageOperation::Generate(spec) => f
                .debug_struct("DecodedIntent")
                .field("kind", &"generate")
                .field("purpose", &self.purpose.purpose.label())
                .field("template_version", &spec.prompt.template_version)
                .field("prompt_chars", &spec.prompt.len_chars())
                .field("references", &spec.references.len())
                .field("expected_source_revision", &self.expected_source_revision)
                .finish(),
            ImageOperation::Edit(spec) => f
                .debug_struct("DecodedIntent")
                .field("kind", &"edit")
                .field("purpose", &self.purpose.purpose.label())
                .field("template_version", &spec.prompt.template_version)
                .field("prompt_chars", &spec.prompt.len_chars())
                .field("references", &spec.references.len())
                .field("expected_source_revision", &self.expected_source_revision)
                .finish(),
            ImageOperation::Transform(spec) => f
                .debug_struct("DecodedIntent")
                .field("kind", &"transform")
                .field("purpose", &self.purpose.purpose.label())
                .field("steps", &spec.steps.len())
                .field("expected_source_revision", &self.expected_source_revision)
                .finish(),
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push(char::from(DIGITS[usize::from(byte >> 4)]));
        text.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    text
}

impl crate::config::ImageConfig {
    /// Digest of the same bounded canonical configuration snapshot retained by ImageService.
    /// It is an identity input, never an authorization or proof of operator propagation.
    pub fn canonical_fingerprint(&self) -> Result<[u8; 32], ConfigError> {
        encode_config(self, crate::service::PROTOCOL_MAX_SNAPSHOT_BYTES)
            .map(|encoded| encoded.fingerprint)
            .map_err(|error| match error {
                CodecError::Config(error) => error,
                _ => ConfigError::InvalidField {
                    field: "config_snapshot",
                    reason: "configuration exceeds canonical protocol bounds",
                },
            })
    }
}
