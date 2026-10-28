//! Private BFL backend: FLUX 3 portrait submit, poll, pinned download.
//!
//! Relocated without behavior change: only the opaque 1024 x 1536 creator
//! portrait profile is admitted, with no reference upload or search. A
//! moderated terminal status is a refusal without download or resubmission.
//! Delivery is credential-free through an allowlisted host; credentials,
//! bodies and URLs never reach diagnostics.

use std::{net::SocketAddr, time::Duration};

use serde_json::{Value, json};

use super::{
    ApiError, GeneratedImage, ImageBackground, ImageEdit, ImageFormat, ImageUsage,
    OperationalBounds, check_credential,
};
use df_providers::{
    EncodedMediaFacts, HttpMethod, HttpRequest, NativeHttp, NativeHttpRefusal, TextSchemaLimits,
    inspect_png_off_worker, parse_unique_json,
};
use super::codec::jpeg_size;

pub(crate) const BFL_FLUX3_IMAGE: &str = "flux-3-image";

/// Fixture socket for loopback tests; `None` selects the live origin.
pub(crate) struct BflConfig {
    fixture: Option<SocketAddr>,
}

impl BflConfig {
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

/// Closed diagnostics: stage, terminal HTTP status, delivery host and the
/// admission reason when one applies. No bodies, URLs or credentials.
#[derive(Debug)]
pub(crate) struct BflFailure {
    pub(crate) stage: &'static str,
    pub(crate) http_status: Option<u16>,
    pub(crate) delivery_host: Option<String>,
    pub(crate) admission_reason: Option<&'static str>,
    pub(crate) error: ApiError,
}

/// Success with actual bytes plus the supplier-reported cost string. The
/// string is a receipt, not a ledger entry; unknown cost stays `None`.
pub(crate) struct BflSuccess {
    pub(crate) image: GeneratedImage,
    pub(crate) credits: Option<String>,
    pub(crate) delivery_host: Option<String>,
}

/// Crate-private executor; owned by the later service worker.
pub(crate) struct BflExecutor {
    origin: String,
    credential: String,
    http: NativeHttp,
    fixture: Option<SocketAddr>,
    bounds: OperationalBounds,
}

impl std::fmt::Debug for BflExecutor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BflExecutor")
            .finish_non_exhaustive()
    }
}

impl BflExecutor {
    pub(crate) fn new(
        config: BflConfig,
        credential: &str,
        bounds: OperationalBounds,
        total_timeout: Duration,
    ) -> Result<Self, ApiError> {
        check_credential(credential)?;
        bounds.validate()?;
        let http = NativeHttp::new(
            df_providers::ProviderLimits::new(total_timeout, 64 * 1024, 64 * 1024).build(),
            config.fixture.is_some(),
        )
        .map_err(|_| ApiError::InvalidRequest)?;
        Ok(Self {
            origin: config.fixture.map_or_else(
                || "https://api.bfl.ai".to_owned(),
                |address| format!("http://{address}"),
            ),
            credential: credential.into(),
            http,
            fixture: config.fixture,
            bounds,
        })
    }

    pub(crate) async fn edit(
        &self,
        edit: &ImageEdit<'_>,
        deadline: Duration,
    ) -> Result<BflSuccess, ApiError> {
        self.edit_diagnosed(edit, deadline)
            .await
            .map_err(|failure| failure.error)
    }

    pub(crate) async fn edit_diagnosed(
        &self,
        edit: &ImageEdit<'_>,
        deadline: Duration,
    ) -> Result<BflSuccess, BflFailure> {
        let mut stage = "validation";
        let mut http_status = None;
        let mut delivery_host = None;
        let mut admission_reason = None;
        self.edit_inner(
            edit,
            deadline,
            &mut stage,
            &mut http_status,
            &mut delivery_host,
            &mut admission_reason,
        )
        .await
        .map_err(|error| BflFailure {
            stage,
            http_status,
            delivery_host,
            admission_reason,
            error,
        })
    }

    async fn edit_inner(
        &self,
        edit: &ImageEdit<'_>,
        deadline: Duration,
        stage: &mut &'static str,
        http_status: &mut Option<u16>,
        delivery_host: &mut Option<String>,
        admission_reason: &mut Option<&'static str>,
    ) -> Result<BflSuccess, ApiError> {
        validate_portrait(edit)?;
        let started = std::time::Instant::now();
        let remaining = || deadline.saturating_sub(started.elapsed());
        let body = generation_body(edit.prompt);
        *stage = "submit";
        let submit = self
            .exchange(
                HttpMethod::Post,
                &format!("{}/v1/flux-3-image", self.origin),
                body.to_string().into_bytes(),
                remaining(),
            )
            .await?;
        *stage = "submit_response";
        let submitted = json_body(&submit)?;
        let credits = submitted
            .get("cost")
            .and_then(Value::as_number)
            .map(ToString::to_string)
            .filter(|value| value.len() <= 32 && !value.starts_with('-'));
        let poll = submitted
            .get("polling_url")
            .and_then(Value::as_str)
            .ok_or(ApiError::Output)?;
        if !poll_allowed(poll, self.fixture) {
            return Err(ApiError::Output);
        }
        loop {
            *stage = "poll";
            let response = self
                .exchange(HttpMethod::Get, poll, vec![], remaining())
                .await?;
            *stage = "poll_response";
            let value = json_body(&response)?;
            match status(&value)? {
                Status::Pending => {
                    let pause = self.bounds.poll_interval.min(remaining());
                    if pause.is_zero() {
                        return Err(ApiError::Transport(NativeHttpRefusal::Deadline));
                    }
                    tokio::time::sleep(pause).await;
                }
                Status::Ready => {
                    let locator = value
                        .get("result")
                        .and_then(|result| result.get("sample"))
                        .and_then(Value::as_str)
                        .ok_or(ApiError::Output)?;
                    *delivery_host = delivery_host_of(locator);
                    *stage = "delivery_admission";
                    let fetch = credential_free_fetch(
                        &self.http,
                        locator,
                        self.bounds.bfl_max_delivery_bytes,
                        remaining().min(Duration::from_secs(30)),
                        self.fixture,
                        http_status,
                    )
                    .await
                    .map_err(|(error, reason)| {
                        *admission_reason = Some(reason);
                        ApiError::Transport(error)
                    })?;
                    *stage = "download";
                    let (bytes, mime) = fetch;
                    *stage = "image_validation";
                    let (bytes, width, height, format, has_alpha_channel) =
                        check_delivery(bytes, &mime).await?;
                    if width > 4096
                        || height > 4096
                        || u64::from(width) * u64::from(height) > 4_194_304
                        || width * 3 != height * 2
                    {
                        return Err(ApiError::Output);
                    }
                    return Ok(BflSuccess {
                        image: GeneratedImage {
                            bytes,
                            format,
                            width,
                            height,
                            has_alpha_channel,
                            usage: ImageUsage::default(),
                        },
                        credits,
                        delivery_host: delivery_host.clone(),
                    });
                }
            }
        }
    }

    async fn exchange(
        &self,
        method: HttpMethod,
        url: &str,
        body: Vec<u8>,
        remaining: Duration,
    ) -> Result<df_providers::NativeHttpResponse, ApiError> {
        let response = self
            .http
            .execute(
                HttpRequest::bfl_native(method, url, &self.credential, body),
                remaining,
                std::future::pending(),
            )
            .await
            .map_err(|failure| ApiError::Transport(failure.reason))?;
        if matches!(response.status, 401 | 403) {
            return Err(ApiError::Unauthorized);
        }
        if matches!(response.status, 200 | 503)
            && let Ok(value) = json_body(&response)
            && matches!(
                value.get("status").and_then(Value::as_str),
                Some("Request Moderated" | "Content Moderated")
            )
        {
            return Err(ApiError::Refused);
        }
        if response.status != 200 {
            return Err(ApiError::Provider(response.status));
        }
        Ok(response)
    }
}

fn validate_portrait(edit: &ImageEdit<'_>) -> Result<(), ApiError> {
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
    if !edit.references.is_empty()
        || edit.background != ImageBackground::Opaque
        || edit.width != 1024
        || edit.height != 1536
    {
        return Err(ApiError::InvalidRequest);
    }
    Ok(())
}

fn generation_body(prompt: &str) -> Value {
    json!({"prompt":prompt,"aspect_ratio":"2:3","resolution":"1.5k","grounding":false,"safety_tolerance":2,"version":"latest"})
}

fn poll_allowed(locator: &str, fixture: Option<SocketAddr>) -> bool {
    if locator.contains(['\\', '\0']) || locator.len() > 8192 || locator.contains('#') {
        return false;
    }
    if locator.contains('@') {
        return false;
    }
    if let Some(address) = fixture {
        let prefix = format!("http://{address}/v1/get_result");
        return locator.starts_with(&prefix);
    }
    if !locator.starts_with("https://") {
        return false;
    }
    let Some(rest) = locator.strip_prefix("https://") else {
        return false;
    };
    let Some((host, path)) = rest.split_once('/') else {
        return false;
    };
    if host.contains(':') {
        return false;
    }
    if path != "v1/get_result?id=x" && !path.starts_with("v1/get_result?") {
        let path_only = path.split('?').next().unwrap_or(path);
        if path_only != "v1/get_result" {
            return false;
        }
    }
    host == "api.bfl.ai" || (host.starts_with("api.") && host.ends_with(".bfl.ai"))
}

fn delivery_host_of(locator: &str) -> Option<String> {
    let rest = locator.strip_prefix("https://")?;
    let (host, _) = rest.split_once('/')?;
    if host.contains(':') || host.contains('@') || host.contains('\\') || host.contains('\0') {
        return None;
    }
    if host.ends_with(".bfl.ai") && host.len() <= 253 {
        Some(host.to_owned())
    } else {
        None
    }
}

async fn check_delivery(
    bytes: Vec<u8>,
    mime: &str,
) -> Result<(Vec<u8>, u32, u32, ImageFormat, bool), ApiError> {
    if mime == "image/png" {
        let (bytes, EncodedMediaFacts::Png { width, height }) =
            inspect_png_off_worker(bytes).await.map_err(|_| ApiError::Output)?
        else {
            return Err(ApiError::Output);
        };
        let has_alpha_channel = matches!(bytes.get(25), Some(4 | 6));
        Ok((bytes, width, height, ImageFormat::Png, has_alpha_channel))
    } else if mime == "image/jpeg" || mime == "image/jpg" {
        let (width, height) = jpeg_size(&bytes).ok_or(ApiError::Output)?;
        Ok((bytes, width, height, ImageFormat::Jpeg, false))
    } else {
        Err(ApiError::Output)
    }
}

async fn credential_free_fetch(
    http: &NativeHttp,
    locator: &str,
    max_bytes: usize,
    remaining: Duration,
    fixture: Option<SocketAddr>,
    http_status: &mut Option<u16>,
) -> Result<(Vec<u8>, String), (NativeHttpRefusal, &'static str)> {
    if fixture.is_some() {
        let allowed = locator.starts_with(&format!(
            "http://{}/",
            fixture.expect("fixture carries an address")
        ));
        if !allowed {
            return Err((NativeHttpRefusal::InvalidEndpoint, "url_contract"));
        }
    } else if delivery_host_of(locator).is_none() {
        return Err((NativeHttpRefusal::InvalidEndpoint, "url_contract"));
    }
    let (bytes, mime, observed) =
        df_providers::fetch_bfl_delivery(http, locator, max_bytes, remaining, fixture).await?;
    if let Some(status) = observed {
        *http_status = Some(status);
    }
    Ok((bytes, mime))
}

fn json_body(response: &df_providers::NativeHttpResponse) -> Result<Value, ApiError> {
    parse_unique_json(
        &response.body,
        TextSchemaLimits {
            maximum_schema_bytes: 64 * 1024,
            maximum_depth: 16,
            maximum_nodes: 2048,
        },
    )
    .map_err(|_| ApiError::Output)
}

#[derive(Debug, PartialEq)]
enum Status {
    Pending,
    Ready,
}

fn status(value: &Value) -> Result<Status, ApiError> {
    match value.get("status").and_then(Value::as_str) {
        Some("Pending" | "Reasoning" | "Generating") => Ok(Status::Pending),
        Some("Ready") => Ok(Status::Ready),
        Some("Request Moderated" | "Content Moderated") => Err(ApiError::Refused),
        Some("Error" | "Task not found") => Err(ApiError::Output),
        _ => Err(ApiError::Output),
    }
}
