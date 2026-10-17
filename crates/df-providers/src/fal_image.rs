//! Photographic images through fal's queue (owner's choice, 2026-10-17: only FLUX read as a real
//! photograph). An [`ImageEdit`] with no references is generated from text by FLUX 1.1 Pro Ultra in
//! raw mode; one with references is a multi-reference FLUX Kontext Max edit; a transparent
//! cut-out is then passed through BiRefNet background removal. Every call asks for `sync_mode`, so
//! the image comes back inside the result as a data URI and nothing is fetched from a media host.
//! One submit, status reads until completion or the deadline, one result read; never retried.

use std::{
    net::SocketAddr,
    time::{Duration, Instant},
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};

use crate::image_edit::{jpeg_size, webp_facts};
use crate::media_container::{EncodedMediaFacts, inspect_png};
use crate::native_http::{NativeHttp, NativeHttpResponse};
use crate::text_schema::{TextSchemaLimits, parse_unique_json};
use crate::{
    GeneratedImage, HttpMethod, HttpRequest, ImageBackground, ImageEdit, ImageEditError,
    ImageFormat, ImageModeration, ImageUsage, NativeHttpLimits, NativeHttpRefusal,
};

const QUEUE_ORIGIN: &str = "https://queue.fal.run";
/// Text to image: FLUX 1.1 Pro Ultra, `raw` for an unprocessed camera look.
pub const FLUX_ULTRA: &str = "fal-ai/flux-pro/v1.1-ultra";
/// Several reference photographs edited into one picture.
pub const FLUX_KONTEXT_MULTI: &str = "fal-ai/flux-pro/kontext/max/multi";
/// Background removal for transparent cut-outs.
pub const BACKGROUND_REMOVAL: &str = "fal-ai/birefnet/v2";
const MAXIMUM_REQUEST_BYTES: usize = 16 * 1024 * 1024;
const MAXIMUM_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
const MAXIMUM_IMAGE_BYTES: usize = 11 * 1024 * 1024;
const POLL_INTERVAL: Duration = Duration::from_millis(500);
/// The aspect ratios FLUX accepts, as (name, width / height).
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

/// Where the adapter sends calls.
pub struct FalImageConfig {
    fixture: Option<SocketAddr>,
}

impl FalImageConfig {
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

pub struct NativeFalImageProvider {
    origin: String,
    credential: String,
    http: NativeHttp,
}

impl std::fmt::Debug for NativeFalImageProvider {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NativeFalImageProvider")
            .finish_non_exhaustive()
    }
}

impl NativeFalImageProvider {
    /// `total_timeout` bounds each HTTP exchange; [`NativeFalImageProvider::edit`]'s deadline
    /// bounds the whole call.
    pub fn new(
        config: FalImageConfig,
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
            origin: config.fixture.map_or_else(
                || QUEUE_ORIGIN.to_owned(),
                |address| format!("http://{address}"),
            ),
            credential: credential.to_owned(),
            http,
        })
    }

    /// Makes the picture `edit` asks for: generated from text when it has no references, edited
    /// from them otherwise, and cut out when it asks for a transparent background. `deadline` is
    /// the time the whole call (every step) may still take.
    pub async fn edit(
        &self,
        edit: &ImageEdit<'_>,
        deadline: Duration,
    ) -> Result<GeneratedImage, ImageEditError> {
        edit.validate()?;
        let started = Instant::now();
        let remaining = || deadline.saturating_sub(started.elapsed());
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
        let (bytes, format) = self.run(endpoint, &body, remaining()).await?;
        let (bytes, format) = if edit.background == ImageBackground::Transparent {
            let body = json!({
                "image_url": data_uri(format, &bytes),
                "model": "General Use (Heavy)",
                "operating_resolution": "1024x1024",
                "output_format": "png",
                "sync_mode": true,
            });
            self.run(BACKGROUND_REMOVAL, &body, remaining()).await?
        } else {
            (bytes, format)
        };
        let (width, height, has_alpha_channel) = facts(format, &bytes)?;
        if edit.background == ImageBackground::Transparent && !has_alpha_channel {
            return Err(ImageEditError::Output);
        }
        Ok(GeneratedImage {
            bytes,
            format,
            width,
            height,
            has_alpha_channel,
            usage: ImageUsage::default(),
        })
    }

    /// One queued fal call: submit, poll its status, read its result, and decode the first image.
    async fn run(
        &self,
        endpoint: &str,
        body: &Value,
        deadline: Duration,
    ) -> Result<(Vec<u8>, ImageFormat), ImageEditError> {
        let started = Instant::now();
        let remaining = || deadline.saturating_sub(started.elapsed());
        let submitted = self
            .exchange(
                HttpMethod::Post,
                &format!("/{endpoint}"),
                body.to_string().into_bytes(),
                remaining(),
            )
            .await?;
        let queue = queue_reply(&submitted)?;
        loop {
            let status = self
                .exchange(HttpMethod::Get, &queue.status_path, Vec::new(), remaining())
                .await?;
            match status_of(&json_body(&status)?)? {
                Status::Completed => break,
                Status::Waiting => {
                    let pause = POLL_INTERVAL.min(remaining());
                    if pause.is_zero() {
                        return Err(ImageEditError::Transport(NativeHttpRefusal::Deadline));
                    }
                    tokio::time::sleep(pause).await;
                }
            }
        }
        let result = self
            .exchange(HttpMethod::Get, &queue.result_path, Vec::new(), remaining())
            .await?;
        first_image(&json_body(&result)?)
    }

    async fn exchange(
        &self,
        method: HttpMethod,
        path: &str,
        body: Vec<u8>,
        remaining: Duration,
    ) -> Result<NativeHttpResponse, ImageEditError> {
        let request = HttpRequest::fal_native(
            method,
            &format!("{}{path}", self.origin),
            &self.credential,
            body,
        );
        let response = self
            .http
            .execute(request, remaining, std::future::pending())
            .await
            .map_err(|failure| ImageEditError::Transport(failure.reason))?;
        match response.status {
            200 | 202 => Ok(response),
            401 | 403 => Err(ImageEditError::Unauthorized),
            422 if is_content_refusal(&response.body) => Err(ImageEditError::Refused),
            status => Err(ImageEditError::Provider(status)),
        }
    }
}

/// The FLUX aspect ratio closest to `width` by `height`.
fn aspect_ratio(width: u32, height: u32) -> &'static str {
    let wanted = f64::from(width) / f64::from(height.max(1));
    ASPECT_RATIOS
        .iter()
        .min_by(|left, right| (left.1 - wanted).abs().total_cmp(&(right.1 - wanted).abs()))
        .map_or("16:9", |(name, _)| name)
}

/// FLUX's safety tolerance (1 strictest, 6 most permissive) for the content rating.
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
    let images: Vec<String> = edit
        .references
        .iter()
        .map(|reference| data_uri(reference.format, reference.bytes))
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

fn json_body(response: &NativeHttpResponse) -> Result<Value, ImageEditError> {
    parse_unique_json(
        &response.body,
        TextSchemaLimits {
            maximum_schema_bytes: MAXIMUM_RESPONSE_BYTES,
            maximum_depth: 16,
            maximum_nodes: 65_536,
        },
    )
    .map_err(|_| ImageEditError::Output)
}

/// A 422 from fal whose detail names its content checker.
fn is_content_refusal(body: &[u8]) -> bool {
    [&b"content_policy"[..], &b"nsfw"[..], &b"safety"[..]]
        .iter()
        .any(|needle| body.windows(needle.len()).any(|window| window == *needle))
}

struct QueueReply {
    status_path: String,
    result_path: String,
}

fn valid_request_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 256
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

/// The path of `url` on fal's queue host when it names request `id`, ending in nothing or
/// `suffix`; any query, fragment, traversal or other host is refused.
fn confined_path<'a>(url: &'a str, id: &str, suffix: Option<&str>) -> Option<&'a str> {
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

fn queue_reply(response: &NativeHttpResponse) -> Result<QueueReply, ImageEditError> {
    let value = json_body(response)?;
    let id = value
        .get("request_id")
        .and_then(Value::as_str)
        .filter(|id| valid_request_id(id))
        .ok_or(ImageEditError::Output)?;
    let path = |field: &str, suffix: Option<&str>| {
        value
            .get(field)
            .and_then(Value::as_str)
            .and_then(|url| confined_path(url, id, suffix))
            .map(str::to_owned)
            .ok_or(ImageEditError::Output)
    };
    Ok(QueueReply {
        status_path: path("status_url", Some("status"))?,
        result_path: path("response_url", None)?,
    })
}

enum Status {
    Waiting,
    Completed,
}

fn status_of(value: &Value) -> Result<Status, ImageEditError> {
    if value.get("error").is_some_and(|error| !error.is_null()) {
        return Err(ImageEditError::Provider(500));
    }
    match value.get("status").and_then(Value::as_str) {
        Some("IN_QUEUE" | "IN_PROGRESS") => Ok(Status::Waiting),
        Some("COMPLETED") => Ok(Status::Completed),
        Some(_) => Err(ImageEditError::Provider(500)),
        None => Err(ImageEditError::Output),
    }
}

/// The first image of a result (`images[0]` or `image`), returned in-band as a data URI. A
/// result flagged as unsafe content is a refusal.
fn first_image(value: &Value) -> Result<(Vec<u8>, ImageFormat), ImageEditError> {
    if value
        .get("has_nsfw_concepts")
        .and_then(Value::as_array)
        .is_some_and(|flags| flags.iter().any(|flag| flag.as_bool() == Some(true)))
    {
        return Err(ImageEditError::Refused);
    }
    let url = value
        .get("images")
        .and_then(Value::as_array)
        .and_then(|images| images.first())
        .or_else(|| value.get("image"))
        .and_then(|image| image.get("url"))
        .and_then(Value::as_str)
        .ok_or(ImageEditError::Refused)?;
    let (header, payload) = url
        .strip_prefix("data:")
        .and_then(|rest| rest.split_once(";base64,"))
        .ok_or(ImageEditError::Output)?;
    let format = match header {
        "image/jpeg" => ImageFormat::Jpeg,
        "image/png" => ImageFormat::Png,
        "image/webp" => ImageFormat::Webp,
        _ => return Err(ImageEditError::Output),
    };
    let bytes = STANDARD
        .decode(payload)
        .map_err(|_| ImageEditError::Output)?;
    if bytes.is_empty() || bytes.len() > MAXIMUM_IMAGE_BYTES {
        return Err(ImageEditError::Output);
    }
    Ok((bytes, format))
}

/// Width, height and whether the image carries alpha, read from its own header.
fn facts(format: ImageFormat, bytes: &[u8]) -> Result<(u32, u32, bool), ImageEditError> {
    match format {
        ImageFormat::Png => {
            let EncodedMediaFacts::Png { width, height } =
                inspect_png(bytes).map_err(|_| ImageEditError::Output)?
            else {
                return Err(ImageEditError::Output);
            };
            let color_type = bytes.get(25).copied().ok_or(ImageEditError::Output)?;
            Ok((width, height, matches!(color_type, 4 | 6)))
        }
        ImageFormat::Jpeg => jpeg_size(bytes)
            .map(|(width, height)| (width, height, false))
            .ok_or(ImageEditError::Output),
        ImageFormat::Webp => webp_facts(bytes).ok_or(ImageEditError::Output),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_live_provider_starts_within_the_transport_limits() {
        assert!(
            NativeFalImageProvider::new(
                FalImageConfig::live(),
                "key:secret",
                Duration::from_secs(60)
            )
            .is_ok()
        );
    }

    #[test]
    fn sizes_map_to_the_nearest_flux_aspect_ratio() {
        assert_eq!(aspect_ratio(1536, 864), "16:9");
        assert_eq!(aspect_ratio(1024, 1536), "2:3");
        assert_eq!(aspect_ratio(1024, 1024), "1:1");
        assert_eq!(aspect_ratio(2560, 1080), "21:9");
    }

    #[test]
    fn a_result_image_is_read_from_its_data_uri_and_a_flagged_one_is_refused() {
        let jpeg = STANDARD.encode([0xff, 0xd8, 0xff]);
        let value = json!({"images": [{"url": format!("data:image/jpeg;base64,{jpeg}")}]});
        assert_eq!(
            first_image(&value),
            Ok((vec![0xff, 0xd8, 0xff], ImageFormat::Jpeg))
        );
        let flagged =
            json!({"images": [{"url": "data:image/png;base64,AA=="}], "has_nsfw_concepts": [true]});
        assert_eq!(first_image(&flagged), Err(ImageEditError::Refused));
        let hosted = json!({"images": [{"url": "https://v3.fal.media/files/x.jpg"}]});
        assert_eq!(first_image(&hosted), Err(ImageEditError::Output));
        let removed = json!({"image": {"url": format!("data:image/png;base64,{jpeg}")}});
        assert!(first_image(&removed).is_ok());
    }

    #[test]
    fn queue_urls_must_name_this_request_on_the_queue_host() {
        let id = "abc-123";
        let good = format!("{QUEUE_ORIGIN}/fal-ai/flux-pro/requests/{id}/status");
        assert_eq!(
            confined_path(&good, id, Some("status")),
            Some(format!("/fal-ai/flux-pro/requests/{id}/status").as_str())
        );
        assert!(
            confined_path("https://evil.example/fal-ai/x/requests/abc-123", id, None).is_none()
        );
        assert!(confined_path(&format!("{good}?x=1"), id, Some("status")).is_none());
        assert!(
            confined_path(&format!("{QUEUE_ORIGIN}/fal-ai/x/requests/other"), id, None).is_none()
        );
    }

    #[test]
    fn generation_is_raw_ultra_and_kontext_carries_every_reference() {
        let reference = [crate::ReferenceImage {
            format: ImageFormat::Png,
            bytes: b"png",
        }];
        let base = ImageEdit {
            model: crate::ImageEditModel::Gpt25Flare,
            prompt: "a room",
            references: &[],
            width: 1536,
            height: 864,
            quality: crate::ImageQuality::High,
            background: ImageBackground::Opaque,
            output: ImageFormat::Jpeg,
            moderation: ImageModeration::Low,
        };
        let generated = generation_body(&base, ImageFormat::Jpeg);
        assert_eq!(generated["raw"], true);
        assert_eq!(generated["aspect_ratio"], "16:9");
        assert_eq!(generated["sync_mode"], true);
        assert_eq!(generated["safety_tolerance"], "5");
        let edited = kontext_body(
            &ImageEdit {
                references: &reference,
                ..base
            },
            ImageFormat::Png,
        );
        assert_eq!(edited["image_urls"].as_array().map(Vec::len), Some(1));
        assert_eq!(edited["output_format"], "png");
    }
}
