//! OpenAI image editing for runtime scene art.
//!
//! One bounded multipart `POST /v1/images/edits` per call: the references are kept faithful
//! and the prompt says what changes. The adapter never retries, never rewords a prompt after a
//! refusal and never reroutes to another model; the caller owns budget, caching and
//! de-duplication. Prompts, references and result bytes are never logged or placed in errors.
//!
//! Model ids, parameters and token prices were read from the OpenAI documentation on
//! 2026-10-11; see `planning/asset-engine.md`:
//! - <https://developers.openai.com/api/docs/guides/image-generation>
//! - <https://developers.openai.com/api/docs/models/gpt-image-2.5-flare>
//! - <https://developers.openai.com/api/docs/pricing>

use std::{
    collections::hash_map::RandomState,
    fmt,
    hash::{BuildHasher, Hasher},
    net::SocketAddr,
    time::Duration,
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::Value;

use crate::media_container::{EncodedMediaFacts, inspect_png};
use crate::native_http::NativeHttp;
use crate::text_schema::{TextSchemaLimits, parse_unique_json};
use crate::{HttpRequest, NativeHttpLimits, NativeHttpRefusal};

const LIVE_ENDPOINT: &str = "https://api.openai.com/v1/images/edits";
const MAXIMUM_REFERENCES: usize = 4;
const MAXIMUM_REFERENCE_BYTES: usize = 4 * 1024 * 1024;
const MAXIMUM_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
const MAXIMUM_REQUEST_BYTES: usize = 12 * 1024 * 1024;
const MINIMUM_PIXELS: u64 = 655_360;
const MAXIMUM_PIXELS: u64 = 4_194_304;
const JPEG_COMPRESSION: &str = "88";

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

impl ImageQuality {
    const fn wire(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

/// The container of an image: references are sent as they are stored, results come back in the
/// requested one (`Png`, `Jpeg` or `Webp`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImageFormat {
    Png,
    Jpeg,
    Webp,
}

impl ImageFormat {
    const fn mime(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Webp => "image/webp",
        }
    }

    const fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::Webp => "webp",
        }
    }

    const fn wire(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpeg",
            Self::Webp => "webp",
        }
    }
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
    /// The supplier answered with another failure status, or was rate limited.
    Provider(u16),
    Transport(NativeHttpRefusal),
    /// A success answer whose image was absent, undecodable, of another size or not complete.
    Output,
}

/// Where the adapter sends calls.
pub struct ImageEditConfig {
    fixture: Option<SocketAddr>,
}

impl ImageEditConfig {
    pub fn live() -> Self {
        Self { fixture: None }
    }

    /// A literal loopback socket for tests.
    pub fn loopback_fixture(address: SocketAddr) -> Result<Self, ImageEditError> {
        if !address.ip().is_loopback() || address.port() == 0 {
            return Err(ImageEditError::InvalidRequest);
        }
        Ok(Self {
            fixture: Some(address),
        })
    }
}

pub struct NativeImageEditProvider {
    endpoint: String,
    credential: String,
    http: NativeHttp,
}

impl fmt::Debug for NativeImageEditProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeImageEditProvider")
            .finish_non_exhaustive()
    }
}

impl NativeImageEditProvider {
    /// `total_timeout` bounds each call, including the response body.
    pub fn new(
        config: ImageEditConfig,
        credential: &str,
        total_timeout: Duration,
    ) -> Result<Self, ImageEditError> {
        if credential.is_empty()
            || credential.len() > 4096
            || !credential.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
        {
            return Err(ImageEditError::InvalidRequest);
        }
        let limits = NativeHttpLimits {
            connect_timeout: Duration::from_secs(5).min(total_timeout),
            read_timeout: total_timeout,
            total_timeout,
            maximum_request_bytes: MAXIMUM_REQUEST_BYTES,
            maximum_response_bytes: MAXIMUM_RESPONSE_BYTES,
            maximum_header_bytes: 16 * 1024,
            maximum_chunks: 65_536,
        };
        let http = NativeHttp::new(limits, config.fixture.is_some())
            .map_err(|_| ImageEditError::InvalidRequest)?;
        Ok(Self {
            endpoint: config.fixture.map_or_else(
                || LIVE_ENDPOINT.to_owned(),
                |address| format!("http://{address}/v1/images/edits"),
            ),
            credential: credential.to_owned(),
            http,
        })
    }

    /// Edits the references per the prompt; one call, never retried. `deadline` is the time this
    /// call may still take.
    pub async fn edit(
        &self,
        edit: &ImageEdit<'_>,
        deadline: Duration,
    ) -> Result<GeneratedImage, ImageEditError> {
        validate(edit)?;
        let (content_type, body) = multipart(edit);
        let request =
            HttpRequest::bearer_native(&self.endpoint, &self.credential, content_type, body);
        let response = self
            .http
            .execute(request, deadline, std::future::pending())
            .await
            .map_err(|failure| ImageEditError::Transport(failure.reason))?;
        match response.status {
            200 => {}
            401 | 403 => return Err(ImageEditError::Unauthorized),
            400 if is_moderation_block(&response.body) => return Err(ImageEditError::Refused),
            status => return Err(ImageEditError::Provider(status)),
        }
        let value = parse_unique_json(
            &response.body,
            TextSchemaLimits {
                maximum_schema_bytes: MAXIMUM_RESPONSE_BYTES,
                maximum_depth: 32,
                maximum_nodes: 65_536,
            },
        )
        .map_err(|_| ImageEditError::Output)?;
        decode(&value, edit)
    }
}

fn validate(edit: &ImageEdit<'_>) -> Result<(), ImageEditError> {
    let pixels = u64::from(edit.width) * u64::from(edit.height);
    let sides_ok = edit.width.is_multiple_of(16)
        && edit.height.is_multiple_of(16)
        && edit.width.max(edit.height) <= 3840
        && edit.width.max(edit.height) <= edit.width.min(edit.height) * 3;
    let references_ok = !edit.references.is_empty()
        && edit.references.len() <= MAXIMUM_REFERENCES
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

/// The `moderation_blocked` error code (or its older `content_policy_violation` name) in a 400
/// body. The body itself is never kept.
fn is_moderation_block(body: &[u8]) -> bool {
    [&b"moderation_blocked"[..], &b"content_policy_violation"[..]]
        .iter()
        .any(|needle| body.windows(needle.len()).any(|window| window == *needle))
}

/// A random multipart boundary that does not occur in any part.
fn boundary(parts: &[&[u8]]) -> String {
    loop {
        let state = RandomState::new();
        let mut text = String::from("dfimagery");
        for round in 0..2u8 {
            let mut hasher = state.build_hasher();
            hasher.write_u8(round);
            text.push_str(&format!("{:016x}", hasher.finish()));
        }
        let needle = text.as_bytes();
        if !parts
            .iter()
            .any(|part| part.windows(needle.len()).any(|window| window == needle))
        {
            return text;
        }
    }
}

fn multipart(edit: &ImageEdit<'_>) -> (String, Vec<u8>) {
    let size = format!("{}x{}", edit.width, edit.height);
    let mut fields: Vec<(&str, &str)> = vec![
        ("model", edit.model.model_id()),
        ("prompt", edit.prompt),
        ("size", &size),
        ("quality", edit.quality.wire()),
        (
            "background",
            match edit.background {
                ImageBackground::Opaque => "opaque",
                ImageBackground::Transparent => "transparent",
            },
        ),
        ("output_format", edit.output.wire()),
        (
            "moderation",
            match edit.moderation {
                ImageModeration::Auto => "auto",
                ImageModeration::Low => "low",
            },
        ),
        ("n", "1"),
    ];
    if matches!(edit.output, ImageFormat::Jpeg | ImageFormat::Webp) {
        fields.push(("output_compression", JPEG_COMPRESSION));
    }
    let mut candidates: Vec<&[u8]> = fields.iter().map(|(_, value)| value.as_bytes()).collect();
    candidates.extend(edit.references.iter().map(|reference| reference.bytes));
    let boundary = boundary(&candidates);
    let mut body = Vec::new();
    for (name, value) in &fields {
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
            )
            .as_bytes(),
        );
    }
    for (index, reference) in edit.references.iter().enumerate() {
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"image[]\"; \
                 filename=\"reference{index}.{}\"\r\nContent-Type: {}\r\n\r\n",
                reference.format.extension(),
                reference.format.mime()
            )
            .as_bytes(),
        );
        body.extend_from_slice(reference.bytes);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

fn tokens(value: &Value, path: &[&str]) -> u64 {
    path.iter()
        .try_fold(value, |current, key| current.get(key))
        .and_then(Value::as_u64)
        .unwrap_or(0)
}

fn decode(value: &Value, edit: &ImageEdit<'_>) -> Result<GeneratedImage, ImageEditError> {
    let usage = ImageUsage {
        text_input_tokens: tokens(value, &["usage", "input_tokens_details", "text_tokens"]),
        image_input_tokens: tokens(value, &["usage", "input_tokens_details", "image_tokens"]),
        output_tokens: tokens(value, &["usage", "output_tokens"]),
    };
    let encoded = value
        .get("data")
        .and_then(Value::as_array)
        .and_then(|data| data.first())
        .and_then(|item| item.get("b64_json"))
        .and_then(Value::as_str);
    // A success status without an image is the supplier declining without an error code.
    let Some(encoded) = encoded else {
        return Err(ImageEditError::Refused);
    };
    let bytes = STANDARD
        .decode(encoded)
        .map_err(|_| ImageEditError::Output)?;
    let (width, height, has_alpha_channel) = match edit.output {
        ImageFormat::Png => {
            let EncodedMediaFacts::Png { width, height } =
                inspect_png(&bytes).map_err(|_| ImageEditError::Output)?
            else {
                return Err(ImageEditError::Output);
            };
            // The colour type is byte 25: the ninth byte of the IHDR payload `inspect_png` proved.
            let color_type = bytes.get(25).copied().ok_or(ImageEditError::Output)?;
            (width, height, matches!(color_type, 4 | 6))
        }
        ImageFormat::Jpeg => {
            let (width, height) = jpeg_size(&bytes).ok_or(ImageEditError::Output)?;
            (width, height, false)
        }
        ImageFormat::Webp => webp_facts(&bytes).ok_or(ImageEditError::Output)?,
    };
    if (width, height) != (edit.width, edit.height) {
        return Err(ImageEditError::Output);
    }
    if edit.background == ImageBackground::Transparent && !has_alpha_channel {
        return Err(ImageEditError::Output);
    }
    Ok(GeneratedImage {
        bytes,
        format: edit.output,
        width,
        height,
        has_alpha_channel,
        usage,
    })
}

/// Width, height and whether it carries alpha, of a complete WebP: the RIFF container's size
/// matches the file and its first chunk is a lossy (`VP8 `), lossless (`VP8L`) or extended
/// (`VP8X`) image header.
fn webp_facts(bytes: &[u8]) -> Option<(u32, u32, bool)> {
    let u32_le = |at: usize| -> Option<u32> {
        Some(u32::from_le_bytes(
            bytes.get(at..at.checked_add(4)?)?.try_into().ok()?,
        ))
    };
    let u24_le = |at: usize| -> Option<u32> {
        let part = bytes.get(at..at.checked_add(3)?)?;
        Some(u32::from(part[0]) | (u32::from(part[1]) << 8) | (u32::from(part[2]) << 16))
    };
    if bytes.get(..4)? != b"RIFF"
        || bytes.get(8..12)? != b"WEBP"
        || usize::try_from(u32_le(4)?).ok()?.checked_add(8)? != bytes.len()
    {
        return None;
    }
    match bytes.get(12..16)? {
        b"VP8X" => {
            let alpha = bytes.get(20)? & 0x10 != 0;
            Some((u24_le(24)? + 1, u24_le(27)? + 1, alpha))
        }
        b"VP8L" => {
            if *bytes.get(20)? != 0x2f {
                return None;
            }
            let packed = u32_le(21)?;
            Some((
                (packed & 0x3fff) + 1,
                ((packed >> 14) & 0x3fff) + 1,
                (packed >> 28) & 1 == 1,
            ))
        }
        b"VP8 " => {
            if bytes.get(23..26)? != [0x9d, 0x01, 0x2a] {
                return None;
            }
            let width = u32::from(u16::from_le_bytes([*bytes.get(26)?, *bytes.get(27)?]) & 0x3fff);
            let height = u32::from(u16::from_le_bytes([*bytes.get(28)?, *bytes.get(29)?]) & 0x3fff);
            (width > 0 && height > 0).then_some((width, height, false))
        }
        _ => None,
    }
}

/// Width and height of a complete JPEG: it starts with the start-of-image marker, ends with the
/// end-of-image marker and declares a frame.
fn jpeg_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.get(..2)? != [0xff, 0xd8] || bytes.get(bytes.len().checked_sub(2)?..)? != [0xff, 0xd9]
    {
        return None;
    }
    let mut position = 2usize;
    while position + 4 <= bytes.len() {
        if *bytes.get(position)? != 0xff {
            return None;
        }
        let marker = *bytes.get(position + 1)?;
        if marker == 0xff {
            position += 1;
            continue;
        }
        let length = usize::from(u16::from_be_bytes([
            *bytes.get(position + 2)?,
            *bytes.get(position + 3)?,
        ]));
        if matches!(marker, 0xc0..=0xc2) {
            let height = u16::from_be_bytes([*bytes.get(position + 5)?, *bytes.get(position + 6)?]);
            let width = u16::from_be_bytes([*bytes.get(position + 7)?, *bytes.get(position + 8)?]);
            return (width > 0 && height > 0).then_some((u32::from(width), u32::from(height)));
        }
        position = position.checked_add(2)?.checked_add(length)?;
    }
    None
}
