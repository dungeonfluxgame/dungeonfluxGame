//! Private OpenAI image backend: one bounded edit or generation.
//!
//! Relocated from the old provider adapter without behavior change: a call
//! with references is a multipart `POST /v1/images/edits`, without them a
//! JSON `POST /v1/images/generations`. One call, never retried, reworded or
//! rerouted. Prompts, references and bytes never reach logs or errors.

use std::{
    collections::hash_map::RandomState,
    hash::{BuildHasher, Hasher},
    net::SocketAddr,
    time::Duration,
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::Value;

use super::{
    ApiError, GeneratedImage, IMAGE_EDIT_MAXIMUM_PROMPT_CHARACTERS, ImageBackground, ImageEdit,
    ImageFormat, ImageModeration, ImageQuality, ImageUsage, OperationalBounds, check_credential,
};
use super::codec::{jpeg_size, webp_facts};
use df_providers::{
    EncodedMediaFacts, HttpRequest, NativeHttp, NativeHttpLimits, TextSchemaLimits,
    inspect_png_off_worker, parse_unique_json,
};

const LIVE_ENDPOINT: &str = "https://api.openai.com/v1/images/edits";
const LIVE_GENERATIONS_ENDPOINT: &str = "https://api.openai.com/v1/images/generations";
const MAXIMUM_REFERENCES: usize = 4;
const MAXIMUM_REFERENCE_BYTES: usize = 4 * 1024 * 1024;
const MINIMUM_PIXELS: u64 = 655_360;
const MAXIMUM_PIXELS: u64 = 4_194_304;
const JPEG_COMPRESSION: &str = "88";
const JPEG_COMPRESSION_PERCENT: u8 = 88;

/// Fixture socket for loopback tests; `None` selects the live endpoint.
pub(crate) struct OpenAiConfig {
    fixture: Option<SocketAddr>,
}

impl OpenAiConfig {
    pub(crate) fn live() -> Self {
        Self { fixture: None }
    }

    pub(crate) fn loopback_fixture(address: SocketAddr) -> Result<Self, ApiError> {
        if !address.ip().is_loopback() || address.port() == 0 {
            return Err(ApiError::InvalidRequest);
        }
        Ok(Self {
            fixture: Some(address),
        })
    }
}

/// Crate-private executor; owned by the later service worker.
pub(crate) struct OpenAiExecutor {
    endpoint: String,
    generations_endpoint: String,
    credential: String,
    http: NativeHttp,
    bounds: OperationalBounds,
}

impl std::fmt::Debug for OpenAiExecutor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OpenAiExecutor")
            .finish_non_exhaustive()
    }
}

impl OpenAiExecutor {
    pub(crate) fn new(
        config: OpenAiConfig,
        credential: &str,
        bounds: OperationalBounds,
        total_timeout: Duration,
    ) -> Result<Self, ApiError> {
        check_credential(credential)?;
        bounds.validate()?;
        let limits = NativeHttpLimits {
            connect_timeout: Duration::from_secs(5).min(total_timeout),
            read_timeout: total_timeout,
            total_timeout,
            maximum_request_bytes: bounds.openai_max_request_bytes,
            maximum_response_bytes: bounds.openai_max_response_bytes,
            maximum_header_bytes: 16 * 1024,
            maximum_chunks: 65_536,
        };
        let http =
            NativeHttp::new(limits, config.fixture.is_some()).map_err(|_| ApiError::InvalidRequest)?;
        Ok(Self {
            endpoint: config.fixture.map_or_else(
                || LIVE_ENDPOINT.to_owned(),
                |address| format!("http://{address}/v1/images/edits"),
            ),
            generations_endpoint: config.fixture.map_or_else(
                || LIVE_GENERATIONS_ENDPOINT.to_owned(),
                |address| format!("http://{address}/v1/images/generations"),
            ),
            credential: credential.to_owned(),
            http,
            bounds,
        })
    }

    /// One edit or generation within `deadline`; typed outcome or refusal.
    pub(crate) async fn edit(
        &self,
        edit: &ImageEdit<'_>,
        deadline: Duration,
    ) -> Result<GeneratedImage, ApiError> {
        validate(edit)?;
        let (endpoint, content_type, body) = if edit.references.is_empty() {
            (
                &self.generations_endpoint,
                "application/json".to_owned(),
                generation_body(edit),
            )
        } else {
            let (content_type, body) = multipart(edit);
            (&self.endpoint, content_type, body)
        };
        let request = HttpRequest::bearer_native(endpoint, &self.credential, content_type, body);
        let response = self
            .http
            .execute(request, deadline, std::future::pending())
            .await
            .map_err(|failure| ApiError::Transport(failure.reason))?;
        match response.status {
            200 => {}
            401 | 403 => return Err(ApiError::Unauthorized),
            400 if is_moderation_block(&response.body) => return Err(ApiError::Refused),
            429 if is_quota_exhaustion(&response.body) => return Err(ApiError::QuotaExhausted),
            status => return Err(ApiError::Provider(status)),
        }
        let value = parse_unique_json(
            &response.body,
            TextSchemaLimits {
                maximum_schema_bytes: self.bounds.openai_max_response_bytes,
                maximum_depth: 32,
                maximum_nodes: 65_536,
            },
        )
        .map_err(|_| ApiError::Output)?;
        decode(&value, edit).await
    }
}

fn validate(edit: &ImageEdit<'_>) -> Result<(), ApiError> {
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
        || edit.prompt.chars().count() > IMAGE_EDIT_MAXIMUM_PROMPT_CHARACTERS
        || !references_ok
        || !sides_ok
        || !(MINIMUM_PIXELS..=MAXIMUM_PIXELS).contains(&pixels)
        || !output_ok
    {
        return Err(ApiError::InvalidRequest);
    }
    Ok(())
}

fn is_moderation_block(body: &[u8]) -> bool {
    [&b"moderation_blocked"[..], &b"content_policy_violation"[..]]
        .iter()
        .any(|needle| body.windows(needle.len()).any(|window| window == *needle))
}

fn is_quota_exhaustion(body: &[u8]) -> bool {
    serde_json::from_slice::<Value>(body).is_ok_and(|value| {
        let error = &value["error"];
        error["code"]
            .as_str()
            .or_else(|| error["type"].as_str())
            .is_some_and(|reason| {
                matches!(
                    reason,
                    "insufficient_quota" | "billing_hard_limit_reached" | "credit_balance_exhausted"
                )
            })
    })
}

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

fn generation_body(edit: &ImageEdit<'_>) -> Vec<u8> {
    let mut body = serde_json::json!({
        "model": edit.model.model_id(),
        "prompt": edit.prompt,
        "size": format!("{}x{}", edit.width, edit.height),
        "quality": edit.quality.wire(),
        "background": match edit.background {
            ImageBackground::Opaque => "opaque",
            ImageBackground::Transparent => "transparent",
        },
        "output_format": edit.output.wire(),
        "moderation": match edit.moderation {
            ImageModeration::Auto => "auto",
            ImageModeration::Low => "low",
        },
        "n": 1,
    });
    if matches!(edit.output, ImageFormat::Jpeg | ImageFormat::Webp) {
        body["output_compression"] = serde_json::json!(JPEG_COMPRESSION_PERCENT);
    }
    body.to_string().into_bytes()
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
            format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n")
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

async fn decode(value: &Value, edit: &ImageEdit<'_>) -> Result<GeneratedImage, ApiError> {
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
    let Some(encoded) = encoded else {
        return Err(ApiError::Refused);
    };
    let bytes = STANDARD.decode(encoded).map_err(|_| ApiError::Output)?;
    let (bytes, width, height, has_alpha_channel) = match edit.output {
        ImageFormat::Png => {
            let (bytes, EncodedMediaFacts::Png { width, height }) =
                inspect_png_off_worker(bytes).await.map_err(|_| ApiError::Output)?
            else {
                return Err(ApiError::Output);
            };
            let color_type = bytes.get(25).copied().ok_or(ApiError::Output)?;
            (bytes, width, height, matches!(color_type, 4 | 6))
        }
        ImageFormat::Jpeg => {
            let (width, height) = jpeg_size(&bytes).ok_or(ApiError::Output)?;
            (bytes, width, height, false)
        }
        ImageFormat::Webp => {
            let (width, height, has_alpha_channel) =
                webp_facts(&bytes).ok_or(ApiError::Output)?;
            (bytes, width, height, has_alpha_channel)
        }
    };
    if (width, height) != (edit.width, edit.height) {
        return Err(ApiError::Output);
    }
    if edit.background == ImageBackground::Transparent && !has_alpha_channel {
        return Err(ApiError::Output);
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
