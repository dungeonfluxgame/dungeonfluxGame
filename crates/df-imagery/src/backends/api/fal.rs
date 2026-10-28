//! Private fal backend: FLUX generation plus paid alpha extraction.
//!
//! Relocated without behavior change: text-only requests use FLUX 1.1 Pro
//! Ultra in raw mode, reference edits use multi-reference FLUX Kontext Max,
//! and transparent cut-outs pass through BiRefNet. Every call asks for
//! `sync_mode`, so images arrive as data URIs with no media-host fetch.
//!
//! Generation and alpha removal are explicit separately metered stages. An
//! opaque edit performs one paid stage; a transparent edit performs two, and
//! the outcome carries both receipts so no second paid call is hidden.

use std::{collections::HashSet, net::SocketAddr, time::Duration};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};

use super::{
    ApiError, GeneratedImage, ImageBackground, ImageEdit, ImageFormat, ImageModeration, ImageUsage,
    OperationalBounds, check_credential,
};
use super::codec::{jpeg_size, webp_facts};
use df_providers::{
    EncodedMediaFacts, HttpMethod, HttpRequest, NativeHttp, NativeHttpLimits, NativeHttpRefusal,
    TextSchemaLimits, inspect_png_off_worker, parse_unique_json,
};

const QUEUE_ORIGIN: &str = "https://queue.fal.run";
pub(crate) const FLUX_ULTRA: &str = "fal-ai/flux-pro/v1.1-ultra";
pub(crate) const FLUX_KONTEXT_MULTI: &str = "fal-ai/flux-pro/kontext/max/multi";
pub(crate) const BACKGROUND_REMOVAL: &str = "fal-ai/birefnet/v2";
const ASPECT_RATIOS: [(&str, f64); 9] = [
    ("21:9", 21.0 / 9.0),
    ("16:9", 16.0 / 9.0),
    ("3:2", 3.0 / 2.0),
    ("4:3", 4.0 / 3.0),
    ("1:1", 1.0),
    ("3:4", 3.0 / 4.0),
    ("2:3", 2.0 / 3.0),
    ("9:16", 9.0 / 16.0),
    ("9:21", 9.0 / 21.0),
];

/// Fixture socket for loopback tests; `None` selects the live queue origin.
pub(crate) struct FalConfig {
    fixture: Option<SocketAddr>,
}

impl FalConfig {
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

/// Retained facts for one metered fal stage: endpoint, supplier request id
/// when known, and the HTTP status of the terminal exchange.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FalStageReceipt {
    pub(crate) endpoint: &'static str,
    pub(crate) request_id: Option<String>,
    pub(crate) http_status: Option<u16>,
}

/// Receipt for the generation stage alone; bytes are actual decoded output.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FalGenerationReceipt {
    pub(crate) stage: FalStageReceipt,
}

/// Receipt for the paid alpha-extraction stage alone.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FalAlphaReceipt {
    pub(crate) stage: FalStageReceipt,
}

/// Complete transparent-capable outcome with both metering identities.
/// `alpha` is `Some` exactly when the request asked for transparency.
pub(crate) struct FalEditOutcome {
    pub(crate) image: GeneratedImage,
    pub(crate) generation: FalGenerationReceipt,
    pub(crate) alpha: Option<FalAlphaReceipt>,
}

/// Crate-private executor; owned by the later service worker.
pub(crate) struct FalExecutor {
    origin: String,
    credential: String,
    http: NativeHttp,
    bounds: OperationalBounds,
}

impl std::fmt::Debug for FalExecutor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FalExecutor")
            .finish_non_exhaustive()
    }
}

impl FalExecutor {
    pub(crate) fn new(
        config: FalConfig,
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
            maximum_request_bytes: bounds.fal_max_request_bytes,
            maximum_response_bytes: bounds.fal_max_response_bytes,
            maximum_header_bytes: 16 * 1024,
            maximum_chunks: 65_536,
        };
        let http =
            NativeHttp::new(limits, config.fixture.is_some()).map_err(|_| ApiError::InvalidRequest)?;
        Ok(Self {
            origin: config.fixture.map_or_else(
                || QUEUE_ORIGIN.to_owned(),
                |address| format!("http://{address}"),
            ),
            credential: credential.to_owned(),
            http,
            bounds,
        })
    }

    /// Generation stage alone: text or multi-reference FLUX dispatch with
    /// queue submit, status polling and in-band result read.
    pub(crate) async fn run_generation(
        &self,
        edit: &ImageEdit<'_>,
        deadline: Duration,
    ) -> Result<(Vec<u8>, ImageFormat, FalGenerationReceipt), ApiError> {
        validate_generation(edit)?;
        let opaque_format = if edit.output == ImageFormat::Jpeg {
            ImageFormat::Jpeg
        } else {
            ImageFormat::Png
        };
        let (endpoint, body) = if edit.references.is_empty() {
            (FLUX_ULTRA, generation_body(edit, opaque_format))
        } else {
            (FLUX_KONTEXT_MULTI, kontext_body(edit, opaque_format))
        };
        let (bytes, format, receipt) = self.run(endpoint, &body, deadline).await?;
        Ok((
            bytes,
            format,
            FalGenerationReceipt {
                stage: receipt,
            },
        ))
    }

    /// Paid alpha-extraction stage alone: one BiRefNet dispatch over the
    /// generation bytes supplied by the caller.
    pub(crate) async fn run_alpha_removal(
        &self,
        generated: &[u8],
        format: ImageFormat,
        deadline: Duration,
    ) -> Result<(Vec<u8>, ImageFormat, FalAlphaReceipt), ApiError> {
        if generated.is_empty() || generated.len() > self.bounds.fal_max_image_bytes {
            return Err(ApiError::InvalidRequest);
        }
        let body = json!({
            "image_url": data_uri(format, generated),
            "model": "General Use (Heavy)",
            "operating_resolution": "1024x1024",
            "output_format": "png",
            "sync_mode": true,
        });
        let (bytes, format, receipt) = self.run(BACKGROUND_REMOVAL, &body, deadline).await?;
        Ok((bytes, format, FalAlphaReceipt { stage: receipt }))
    }

    /// Full edit preserving legacy behavior: generation, then alpha removal
    /// only for transparent requests, with both receipts retained.
    pub(crate) async fn edit(
        &self,
        edit: &ImageEdit<'_>,
        deadline: Duration,
    ) -> Result<FalEditOutcome, ApiError> {
        let started = std::time::Instant::now();
        let remaining = || deadline.saturating_sub(started.elapsed());
        let (bytes, format, generation) = self.run_generation(edit, remaining()).await?;
        let (bytes, format, alpha) = if edit.background == ImageBackground::Transparent {
            let (bytes, format, receipt) =
                self.run_alpha_removal(&bytes, format, remaining()).await?;
            (bytes, format, Some(receipt))
        } else {
            (bytes, format, None)
        };
        let (bytes, (width, height, has_alpha_channel)) = facts(format, bytes).await?;
        if edit.background == ImageBackground::Transparent && !has_alpha_channel {
            return Err(ApiError::Output);
        }
        Ok(FalEditOutcome {
            image: GeneratedImage {
                bytes,
                format,
                width,
                height,
                has_alpha_channel,
                usage: ImageUsage::default(),
            },
            generation,
            alpha,
        })
    }

    async fn run(
        &self,
        endpoint: &'static str,
        body: &Value,
        deadline: Duration,
    ) -> Result<(Vec<u8>, ImageFormat, FalStageReceipt), ApiError> {
        let started = std::time::Instant::now();
        let remaining = || deadline.saturating_sub(started.elapsed());
        let submitted = self
            .exchange(
                HttpMethod::Post,
                &format!("/{endpoint}"),
                body.to_string().into_bytes(),
                remaining(),
            )
            .await?;
        let queue = queue_reply(&submitted, self.bounds.fal_max_response_bytes)?;
        let mut request_id: Option<String> = None;
        loop {
            let status = self
                .exchange(HttpMethod::Get, &queue.status_path, Vec::new(), remaining())
                .await?;
            let value = json_body(&status, self.bounds.fal_max_response_bytes)?;
            match status_of(&value)? {
                Status::Completed => {
                    request_id = Some(queue.request_id.clone());
                    break;
                }
                Status::Waiting => {
                    let pause = self.bounds.poll_interval.min(remaining());
                    if pause.is_zero() {
                        return Err(ApiError::Transport(NativeHttpRefusal::Deadline));
                    }
                    tokio::time::sleep(pause).await;
                }
            }
        }
        let result = self
            .exchange(HttpMethod::Get, &queue.result_path, Vec::new(), remaining())
            .await?;
        let receipt = FalStageReceipt {
            endpoint,
            request_id,
            http_status: Some(result.status),
        };
        let (bytes, format) = first_image(&json_body(&result, self.bounds.fal_max_response_bytes)?, self.bounds.fal_max_image_bytes)?;
        Ok((bytes, format, receipt))
    }

    async fn exchange(
        &self,
        method: HttpMethod,
        path: &str,
        body: Vec<u8>,
        remaining: Duration,
    ) -> Result<df_providers::NativeHttpResponse, ApiError> {
        let request =
            HttpRequest::fal_native(method, &format!("{}{path}", self.origin), &self.credential, body);
        let response = self
            .http
            .execute(request, remaining, std::future::pending())
            .await
            .map_err(|failure| ApiError::Transport(failure.reason))?;
        match response.status {
            200 | 202 => Ok(response),
            401 | 403 => Err(ApiError::Unauthorized),
            422 if is_content_refusal(&response.body) => Err(ApiError::Refused),
            status => Err(ApiError::Provider(status)),
        }
    }
}

fn validate_generation(edit: &ImageEdit<'_>) -> Result<(), ApiError> {
    let pixels = u64::from(edit.width) * u64::from(edit.height);
    let sides_ok = edit.width.is_multiple_of(16)
        && edit.height.is_multiple_of(16)
        && edit.width.max(edit.height) <= 3840
        && edit.width.max(edit.height) <= edit.width.min(edit.height) * 3;
    let references_ok = edit.references.len() <= 4
        && edit.references.iter().all(|reference| {
            !reference.bytes.is_empty() && reference.bytes.len() <= 4 * 1024 * 1024
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
        || edit.prompt.chars().count() > super::IMAGE_EDIT_MAXIMUM_PROMPT_CHARACTERS
        || !references_ok
        || !sides_ok
        || !(655_360..=4_194_304).contains(&pixels)
        || !output_ok
    {
        return Err(ApiError::InvalidRequest);
    }
    Ok(())
}

fn aspect_ratio(width: u32, height: u32) -> &'static str {
    let wanted = f64::from(width) / f64::from(height.max(1));
    ASPECT_RATIOS
        .iter()
        .min_by(|left, right| (left.1 - wanted).abs().total_cmp(&(right.1 - wanted).abs()))
        .map_or("16:9", |(name, _)| name)
}

fn safety_tolerance(moderation: ImageModeration) -> &'static str {
    match moderation {
        ImageModeration::Low => "5",
        ImageModeration::Auto => "2",
    }
}

fn wire_format(format: ImageFormat) -> &'static str {
    if format == ImageFormat::Jpeg {
        "jpeg"
    } else {
        "png"
    }
}

fn generation_body(edit: &ImageEdit<'_>, format: ImageFormat) -> Value {
    json!({
        "prompt": edit.prompt,
        "raw": true,
        "aspect_ratio": aspect_ratio(edit.width, edit.height),
        "output_format": wire_format(format),
        "safety_tolerance": safety_tolerance(edit.moderation),
        "num_images": 1,
        "sync_mode": true,
    })
}

fn kontext_body(edit: &ImageEdit<'_>, format: ImageFormat) -> Value {
    let mut seen = HashSet::new();
    let images: Vec<String> = edit
        .references
        .iter()
        .map(|reference| data_uri(reference.format, reference.bytes))
        .filter(|image| seen.insert(image.clone()))
        .collect();
    json!({
        "prompt": edit.prompt,
        "image_urls": images,
        "aspect_ratio": aspect_ratio(edit.width, edit.height),
        "output_format": wire_format(format),
        "safety_tolerance": safety_tolerance(edit.moderation),
        "guidance_scale": 3.5,
        "num_images": 1,
        "sync_mode": true,
    })
}

fn data_uri(format: ImageFormat, bytes: &[u8]) -> String {
    format!("data:{};base64,{}", format.mime(), STANDARD.encode(bytes))
}

fn json_body(
    response: &df_providers::NativeHttpResponse,
    max_bytes: usize,
) -> Result<Value, ApiError> {
    parse_unique_json(
        &response.body,
        TextSchemaLimits {
            maximum_schema_bytes: max_bytes,
            maximum_depth: 16,
            maximum_nodes: 65_536,
        },
    )
    .map_err(|_| ApiError::Output)
}

fn is_content_refusal(body: &[u8]) -> bool {
    [&b"content_policy"[..], &b"nsfw"[..], &b"safety"[..]]
        .iter()
        .any(|needle| body.windows(needle.len()).any(|window| window == *needle))
}

struct QueueReply {
    status_path: String,
    result_path: String,
    request_id: String,
}

fn valid_request_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 256
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn confined_path(url: &str, id: &str, suffix: Option<&str>) -> Option<&str> {
    let path = url.strip_prefix(QUEUE_ORIGIN)?;
    let tail = path.strip_prefix('/')?;
    if tail.contains(['?', '#']) || tail.contains("..") {
        return None;
    }
    let needle = format!("/requests/{id}");
    let position = tail.rfind(&needle)?;
    let (app, rest) = (&tail[..position], &tail[position + needle.len()..]);
    let app_ok = app.split('/').count() >= 2 && app.split('/').all(|part| !part.is_empty());
    let rest_ok = match suffix {
        Some(expected) => rest == format!("/{expected}"),
        None => rest.is_empty() || rest == "/response",
    };
    (app_ok && rest_ok).then_some(path)
}

fn queue_reply(
    response: &df_providers::NativeHttpResponse,
    max_bytes: usize,
) -> Result<QueueReply, ApiError> {
    let value = json_body(response, max_bytes)?;
    let id = value
        .get("request_id")
        .and_then(Value::as_str)
        .filter(|id| valid_request_id(id))
        .ok_or(ApiError::Output)?;
    let path = |field: &str, suffix: Option<&str>| {
        value
            .get(field)
            .and_then(Value::as_str)
            .and_then(|url| confined_path(url, id, suffix))
            .map(str::to_owned)
            .ok_or(ApiError::Output)
    };
    Ok(QueueReply {
        status_path: path("status_url", Some("status"))?,
        result_path: path("response_url", None)?,
        request_id: id.to_owned(),
    })
}

enum Status {
    Waiting,
    Completed,
}

fn status_of(value: &Value) -> Result<Status, ApiError> {
    if value.get("error").is_some_and(|error| !error.is_null()) {
        return Err(ApiError::Provider(500));
    }
    match value.get("status").and_then(Value::as_str) {
        Some("IN_QUEUE" | "IN_PROGRESS") => Ok(Status::Waiting),
        Some("COMPLETED") => Ok(Status::Completed),
        Some(_) => Err(ApiError::Provider(500)),
        None => Err(ApiError::Output),
    }
}

fn first_image(value: &Value, max_image_bytes: usize) -> Result<(Vec<u8>, ImageFormat), ApiError> {
    if value
        .get("has_nsfw_concepts")
        .and_then(Value::as_array)
        .is_some_and(|flags| flags.iter().any(|flag| flag.as_bool() == Some(true)))
    {
        return Err(ApiError::Refused);
    }
    let url = value
        .get("images")
        .and_then(Value::as_array)
        .and_then(|images| images.first())
        .or_else(|| value.get("image"))
        .and_then(|image| image.get("url"))
        .and_then(Value::as_str)
        .ok_or(ApiError::Refused)?;
    let (header, payload) = url
        .strip_prefix("data:")
        .and_then(|rest| rest.split_once(";base64,"))
        .ok_or(ApiError::Output)?;
    let format = match header {
        "image/jpeg" => ImageFormat::Jpeg,
        "image/png" => ImageFormat::Png,
        "image/webp" => ImageFormat::Webp,
        _ => return Err(ApiError::Output),
    };
    let bytes = STANDARD.decode(payload).map_err(|_| ApiError::Output)?;
    if bytes.is_empty() || bytes.len() > max_image_bytes {
        return Err(ApiError::Output);
    }
    Ok((bytes, format))
}

async fn facts(
    format: ImageFormat,
    bytes: Vec<u8>,
) -> Result<(Vec<u8>, (u32, u32, bool)), ApiError> {
    match format {
        ImageFormat::Png => {
            let (bytes, EncodedMediaFacts::Png { width, height }) =
                inspect_png_off_worker(bytes).await.map_err(|_| ApiError::Output)?
            else {
                return Err(ApiError::Output);
            };
            let color_type = bytes.get(25).copied().ok_or(ApiError::Output)?;
            Ok((bytes, (width, height, matches!(color_type, 4 | 6))))
        }
        ImageFormat::Jpeg => {
            let (width, height) = jpeg_size(&bytes).ok_or(ApiError::Output)?;
            Ok((bytes, (width, height, false)))
        }
        ImageFormat::Webp => {
            let facts = webp_facts(&bytes).ok_or(ApiError::Output)?;
            Ok((bytes, facts))
        }
    }
}
