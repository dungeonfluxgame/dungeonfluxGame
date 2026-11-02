//! Private BFL backend: FLUX 3 portrait submit, poll, pinned download.
//!
//! Relocated without behavior change: only the opaque 1024 x 1536 creator
//! portrait profile is admitted, with no reference upload or search. A
//! moderated terminal status is a refusal without download or resubmission.
//! Delivery is credential-free through an allowlisted host; credentials,
//! bodies and URLs never reach diagnostics.

use std::{net::SocketAddr, time::Duration};

use serde_json::{Value, json};

use super::codec::jpeg_size;
use super::{
    ApiError, ApiExchange, ApiFailure, DeclineContract, GeneratedImage, ImageBackground, ImageEdit,
    ImageFormat, ImageUsage, OperationalBounds, check_credential, check_credentialed_origin,
    check_endpoint_url, check_model_slug,
};
use df_providers::{
    EncodedMediaFacts, HttpMethod, HttpRequest, NativeHttp, NativeHttpRefusal, TextSchemaLimits,
    inspect_png_off_worker, parse_unique_json,
};

pub(crate) const BFL_FLUX3_IMAGE: &str = "flux-3-image";
pub(crate) const LIVE_BFL_ORIGIN: &str = "https://api.bfl.ai";

/// Explicit origin and model-route selection. Origins are the credential
/// destination (`live`), a loopback fixture socket, or a validated operator
/// base URL. The model route is a configurable nonsecret string defaulting
/// to the supported FLUX route; no arbitrary model capability is claimed.
/// URLs never appear in diagnostics.
pub(crate) struct BflConfig {
    origin: String,
    allow_http: bool,
    fixture: Option<SocketAddr>,
    model_route: String,
}

impl BflConfig {
    pub(crate) fn live() -> Self {
        Self {
            origin: LIVE_BFL_ORIGIN.to_owned(),
            allow_http: false,
            fixture: None,
            model_route: BFL_FLUX3_IMAGE.to_owned(),
        }
    }

    #[cfg(test)]
    pub(crate) fn loopback_fixture(address: SocketAddr) -> Result<Self, ApiError> {
        if !address.ip().is_loopback() || address.port() == 0 {
            return Err(ApiError::InvalidRequest);
        }
        Ok(Self {
            origin: format!("http://{address}"),
            allow_http: true,
            fixture: Some(address),
            model_route: BFL_FLUX3_IMAGE.to_owned(),
        })
    }

    pub(crate) fn operator(origin_url: &str) -> Result<Self, ApiError> {
        check_credentialed_origin(origin_url, "api.bfl.ai")?;
        let origin = canonical_origin(origin_url)?;
        let allow_http = origin.starts_with("http://");
        Ok(Self {
            origin,
            allow_http,
            fixture: None,
            model_route: BFL_FLUX3_IMAGE.to_owned(),
        })
    }

    pub(crate) fn with_route(mut self, route: &str) -> Result<Self, ApiError> {
        check_model_slug(route)?;
        if route.contains('/') {
            return Err(ApiError::InvalidRequest);
        }
        self.model_route = route.to_owned();
        Ok(self)
    }
}

impl std::fmt::Debug for BflConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BflConfig")
            .field("model_route", &self.model_route)
            .finish_non_exhaustive()
    }
}

/// Durable submit checkpoint for one BFL portrait. The runner persists this
/// validated receipt before polling. `origin` binds the configured
/// credential destination, `model_route` binds the selected route, and
/// `request_id` is the bounded opaque supplier correlation extracted from
/// the polling locator; the locator itself is revalidated on resume.
/// Locators never appear in diagnostics.
#[derive(Clone, Eq, PartialEq)]
pub(crate) struct BflPortraitCheckpoint {
    pub(crate) origin: String,
    pub(crate) model_route: String,
    pub(crate) polling_url: String,
    pub(crate) request_id: String,
    pub(crate) credits: Option<String>,
}

impl std::fmt::Debug for BflPortraitCheckpoint {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BflPortraitCheckpoint")
            .field("has_credits", &self.credits.is_some())
            .finish_non_exhaustive()
    }
}

impl BflPortraitCheckpoint {
    pub(crate) fn encode(&self) -> String {
        format!(
            "bfl-portrait-v2|{}|{}|{}|{}|{}",
            self.origin,
            self.model_route,
            self.polling_url,
            self.request_id,
            self.credits.as_deref().unwrap_or("")
        )
    }

    pub(crate) fn decode(encoded: &str) -> Result<Self, ApiError> {
        if encoded.is_empty() || encoded.len() > 24576 {
            return Err(ApiError::InvalidRequest);
        }
        let mut parts = encoded.split('|');
        match (
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
        ) {
            (
                Some("bfl-portrait-v2"),
                Some(origin),
                Some(model_route),
                Some(polling_url),
                Some(request_id),
                Some(credits),
                None,
            ) => {
                if origin.is_empty() || origin.len() > 8192 {
                    return Err(ApiError::InvalidRequest);
                }
                let _ = check_endpoint_url(origin)?;
                check_model_slug(model_route)?;
                if model_route.contains('/') {
                    return Err(ApiError::InvalidRequest);
                }
                if polling_url.is_empty() || polling_url.len() > 8192 {
                    return Err(ApiError::InvalidRequest);
                }
                if !valid_poll_shape(polling_url, origin) || !valid_request_id(request_id) {
                    return Err(ApiError::InvalidRequest);
                }
                if !poll_id_matches(polling_url, request_id) {
                    return Err(ApiError::InvalidRequest);
                }
                let credits = if credits.is_empty() {
                    None
                } else {
                    if !valid_credit_receipt(credits) {
                        return Err(ApiError::InvalidRequest);
                    }
                    Some(credits.to_owned())
                };
                Ok(Self {
                    origin: canonical_origin(origin)?,
                    model_route: model_route.to_owned(),
                    polling_url: polling_url.to_owned(),
                    request_id: request_id.to_owned(),
                    credits,
                })
            }
            _ => Err(ApiError::InvalidRequest),
        }
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
    pub(crate) observation: ApiFailure,
}

struct BflDiagnostics {
    stage: &'static str,
    http_status: Option<u16>,
    delivery_host: Option<String>,
    admission_reason: Option<&'static str>,
}

impl BflDiagnostics {
    fn new(stage: &'static str) -> Self {
        Self {
            stage,
            http_status: None,
            delivery_host: None,
            admission_reason: None,
        }
    }

    fn failure(self, observation: ApiFailure) -> BflFailure {
        BflFailure {
            stage: self.stage,
            http_status: observation
                .response
                .as_ref()
                .map(|response| response.status)
                .or(self.http_status),
            delivery_host: self.delivery_host,
            admission_reason: self.admission_reason,
            observation,
        }
    }
}

/// Success with actual bytes plus the supplier-reported cost string. The
/// string is a receipt, not a ledger entry; unknown cost stays `None`.
pub(crate) struct BflSuccess {
    pub(crate) image: GeneratedImage,
    pub(crate) credits: Option<String>,
    pub(crate) delivery_host: Option<String>,
}

impl std::fmt::Debug for BflSuccess {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BflSuccess")
            .field("byte_length", &self.image.bytes.len())
            .field("format", &self.image.format)
            .field("width", &self.image.width)
            .field("height", &self.image.height)
            .field("has_alpha_channel", &self.image.has_alpha_channel)
            .field("credits", &self.credits)
            .field("delivery_host", &self.delivery_host)
            .finish_non_exhaustive()
    }
}

/// Crate-private executor; owned by the later service worker.
pub(crate) struct BflExecutor {
    origin: String,
    credential: String,
    http: NativeHttp,
    fixture: Option<SocketAddr>,
    bounds: OperationalBounds,
    model_route: String,
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
        check_model_slug(&config.model_route)?;
        let http = NativeHttp::new(
            df_providers::ProviderLimits::new(
                total_timeout.min(super::HTTP_TIMEOUT_CEILING),
                64 * 1024,
                64 * 1024,
            )
            .build(),
            config.allow_http,
        )
        .map_err(|_| ApiError::InvalidRequest)?;
        Ok(Self {
            origin: config.origin,
            credential: credential.into(),
            http,
            fixture: config.fixture,
            bounds,
            model_route: config.model_route,
        })
    }

    /// Paid submit for the portrait stage: validates, POSTs once and returns
    /// the durable checkpoint. A transport failure or malformed receipt after
    /// the POST is unknown to the service, never proven unsent: the runner
    /// persists nothing and must not auto-resubmit.
    #[cfg(test)]
    pub(crate) async fn submit_portrait(
        &self,
        edit: &ImageEdit<'_>,
        deadline: Duration,
    ) -> Result<BflPortraitCheckpoint, ApiError> {
        self.submit_portrait_observed(edit, deadline)
            .await
            .map_err(|failure| failure.error)
    }
    pub(crate) async fn submit_portrait_observed(
        &self,
        edit: &ImageEdit<'_>,
        deadline: Duration,
    ) -> Result<BflPortraitCheckpoint, ApiFailure> {
        validate_portrait(edit).map_err(ApiFailure::before_transport)?;
        let started = std::time::Instant::now();
        let body = generation_body(edit.prompt);
        let submit = self
            .exchange(
                HttpMethod::Post,
                &format!("{}/v1/{}", self.origin, self.model_route),
                body.to_string().into_bytes(),
                deadline.saturating_sub(started.elapsed()),
                ApiExchange::Submit,
            )
            .await?;
        let parsed = (|| -> Result<BflPortraitCheckpoint, ApiError> {
            let submitted = json_body(&submit)?;
            if submitted
                .get("polling_url")
                .is_some_and(|value| !value.is_string())
            {
                return Err(ApiError::Output);
            }
            let poll = submitted
                .get("polling_url")
                .and_then(Value::as_str)
                .ok_or(ApiError::Output)?;
            if !self.poll_locator_allowed(poll) {
                return Err(ApiError::Output);
            }
            let credits = match submitted.get("cost") {
                None | Some(Value::Null) => None,
                Some(Value::Number(cost)) => {
                    let text = cost.to_string();
                    if !valid_credit_receipt(&text) {
                        return Err(ApiError::Output);
                    }
                    Some(text)
                }
                Some(_) => return Err(ApiError::Output),
            };
            let request_id = parse_poll_request_id(poll).ok_or(ApiError::Output)?;
            if credits.is_some() && !valid_credit_receipt(credits.as_deref().unwrap_or("")) {
                return Err(ApiError::Output);
            }
            Ok(BflPortraitCheckpoint {
                origin: self.origin.clone(),
                model_route: self.model_route.clone(),
                polling_url: poll.to_owned(),
                request_id: request_id.to_owned(),
                credits,
            })
        })();
        parsed.map_err(|error| {
            ApiFailure::response(
                error,
                ApiExchange::Submit,
                DeclineContract::BflImages429V1,
                submit,
            )
        })
    }

    fn check_checkpoint(&self, checkpoint: &BflPortraitCheckpoint) -> Result<(), ApiError> {
        if canonical_origin(&checkpoint.origin)? != self.origin {
            return Err(ApiError::InvalidRequest);
        }
        if checkpoint.model_route != self.model_route {
            return Err(ApiError::InvalidRequest);
        }
        if checkpoint
            .credits
            .as_deref()
            .is_some_and(|value| !valid_credit_receipt(value))
        {
            return Err(ApiError::InvalidRequest);
        }
        if !self.poll_locator_allowed(&checkpoint.polling_url) {
            return Err(ApiError::InvalidRequest);
        }
        if !poll_id_matches(&checkpoint.polling_url, &checkpoint.request_id) {
            return Err(ApiError::InvalidRequest);
        }
        Ok(())
    }

    /// Resume a persisted portrait checkpoint with a fresh executor. Never
    /// POSTs: polls the retained locator and downloads the result only. The
    /// origin, route, correlation and locator are validated against this
    /// executor before any egress.
    #[cfg(test)]
    pub(crate) async fn resume_portrait(
        &self,
        checkpoint: &BflPortraitCheckpoint,
        deadline: Duration,
    ) -> Result<BflSuccess, ApiError> {
        self.resume_portrait_diagnosed(checkpoint, deadline)
            .await
            .map_err(|failure| failure.observation.error)
    }

    /// Diagnosed resume carrying the failure stage. Never POSTs.
    pub(crate) async fn resume_portrait_diagnosed(
        &self,
        checkpoint: &BflPortraitCheckpoint,
        deadline: Duration,
    ) -> Result<BflSuccess, BflFailure> {
        self.check_checkpoint(checkpoint).map_err(|error| {
            BflDiagnostics::new("checkpoint").failure(ApiFailure::unknown(error, ApiExchange::Poll))
        })?;
        let mut diagnostics = BflDiagnostics::new("poll");
        self.poll_and_download_inner(
            &checkpoint.polling_url,
            checkpoint.credits.clone(),
            deadline,
            &mut diagnostics,
        )
        .await
        .map_err(|error| diagnostics.failure(error))
    }

    #[cfg(test)]
    pub(crate) async fn edit(
        &self,
        edit: &ImageEdit<'_>,
        deadline: Duration,
    ) -> Result<BflSuccess, ApiError> {
        self.edit_diagnosed(edit, deadline)
            .await
            .map_err(|failure| failure.observation.error)
    }

    #[cfg(test)]
    pub(crate) async fn edit_diagnosed(
        &self,
        edit: &ImageEdit<'_>,
        deadline: Duration,
    ) -> Result<BflSuccess, BflFailure> {
        let mut diagnostics = BflDiagnostics::new("validation");
        self.edit_inner(edit, deadline, &mut diagnostics)
            .await
            .map_err(|error| diagnostics.failure(error))
    }

    #[cfg(test)]
    async fn edit_inner(
        &self,
        edit: &ImageEdit<'_>,
        deadline: Duration,
        diagnostics: &mut BflDiagnostics,
    ) -> Result<BflSuccess, ApiFailure> {
        validate_portrait(edit).map_err(ApiFailure::before_transport)?;
        let started = std::time::Instant::now();
        diagnostics.stage = "submit";
        let checkpoint = self
            .submit_portrait_observed(edit, deadline.saturating_sub(started.elapsed()))
            .await
            .inspect_err(|error| {
                if matches!(error.error, ApiError::Output) {
                    diagnostics.stage = "submit_response";
                }
            })?;
        self.poll_and_download_inner(
            &checkpoint.polling_url,
            checkpoint.credits,
            deadline.saturating_sub(started.elapsed()),
            diagnostics,
        )
        .await
    }

    async fn poll_and_download_inner(
        &self,
        polling_url: &str,
        credits: Option<String>,
        deadline: Duration,
        diagnostics: &mut BflDiagnostics,
    ) -> Result<BflSuccess, ApiFailure> {
        let started = std::time::Instant::now();
        let remaining = || deadline.saturating_sub(started.elapsed());
        loop {
            diagnostics.stage = "poll";
            let response = self
                .exchange(
                    HttpMethod::Get,
                    polling_url,
                    vec![],
                    remaining(),
                    ApiExchange::Poll,
                )
                .await?;
            diagnostics.stage = "poll_response";
            let interpreted = (|| -> Result<Option<String>, ApiError> {
                let value = json_body(&response)?;
                match status(&value)? {
                    Status::Pending => Ok(None),
                    Status::Ready => value
                        .get("result")
                        .and_then(|result| result.get("sample"))
                        .and_then(Value::as_str)
                        .map(|locator| Some(locator.to_owned()))
                        .ok_or(ApiError::Output),
                }
            })()
            .map_err(|error| {
                ApiFailure::response(
                    error,
                    ApiExchange::Poll,
                    DeclineContract::BflImages429V1,
                    response,
                )
            })?;
            match interpreted {
                None => {
                    let pause = self.bounds.poll_interval.min(remaining());
                    if pause.is_zero() {
                        return Err(ApiFailure::accepted(
                            ApiError::Transport(NativeHttpRefusal::Deadline),
                            ApiExchange::Poll,
                        ));
                    }
                    tokio::time::sleep(pause).await;
                }
                Some(locator) => {
                    diagnostics.delivery_host = delivery_host_of(&locator);
                    diagnostics.stage = "delivery_admission";
                    let fetch = credential_free_fetch(
                        &self.http,
                        &locator,
                        self.bounds.bfl_max_delivery_bytes,
                        remaining().min(Duration::from_secs(30)),
                        self.fixture,
                        &mut diagnostics.http_status,
                    )
                    .await
                    .map_err(|(error, reason)| {
                        diagnostics.admission_reason = Some(reason);
                        ApiFailure::accepted(ApiError::Transport(error), ApiExchange::Delivery)
                    })?;
                    diagnostics.stage = "download";
                    let (bytes, mime) = fetch;
                    diagnostics.stage = "image_validation";
                    let (bytes, width, height, format, has_alpha_channel) =
                        check_delivery(bytes, &mime)
                            .await
                            .map_err(|error| ApiFailure::accepted(error, ApiExchange::Delivery))?;
                    if width > 4096
                        || height > 4096
                        || u64::from(width) * u64::from(height) > 4_194_304
                        || width * 3 != height * 2
                    {
                        return Err(ApiFailure::accepted(
                            ApiError::Output,
                            ApiExchange::Delivery,
                        ));
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
                        delivery_host: diagnostics.delivery_host.clone(),
                    });
                }
            }
        }
    }

    fn poll_locator_allowed(&self, locator: &str) -> bool {
        poll_allowed(locator, &self.origin)
    }

    async fn exchange(
        &self,
        method: HttpMethod,
        url: &str,
        body: Vec<u8>,
        remaining: Duration,
        exchange: ApiExchange,
    ) -> Result<df_providers::NativeHttpResponse, ApiFailure> {
        let response = self
            .http
            .execute(
                HttpRequest::bfl_native(method, url, &self.credential, body),
                remaining,
                std::future::pending(),
            )
            .await
            .map_err(|failure| ApiFailure::transport(failure.reason, exchange))?;
        let error = if matches!(response.status, 401 | 403) {
            Some(ApiError::Unauthorized)
        } else if matches!(response.status, 200 | 503)
            && json_body(&response).is_ok_and(|value| {
                matches!(
                    value.get("status").and_then(Value::as_str),
                    Some("Request Moderated" | "Content Moderated")
                )
            })
        {
            Some(ApiError::Refused)
        } else if response.status == 429 {
            Some(ApiError::RateLimited)
        } else if response.status != 200 {
            Some(ApiError::Provider(response.status))
        } else {
            None
        };
        match error {
            Some(error) => Err(ApiFailure::response(
                error,
                exchange,
                DeclineContract::BflImages429V1,
                response,
            )),
            None => Ok(response),
        }
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

fn canonical_origin(origin: &str) -> Result<String, ApiError> {
    check_endpoint_url(origin)?;
    let parsed = url::Url::parse(origin).map_err(|_| ApiError::InvalidRequest)?;
    Ok(parsed.as_str().trim_end_matches('/').to_owned())
}

fn same_network_origin(left: &url::Url, right: &url::Url) -> bool {
    left.scheme() == right.scheme()
        && left.host() == right.host()
        && left.port_or_known_default() == right.port_or_known_default()
}

fn poll_allowed(locator: &str, origin: &str) -> bool {
    if !valid_poll_shape(locator, origin) {
        return false;
    }
    let (Ok(locator), Ok(origin)) = (url::Url::parse(locator), url::Url::parse(origin)) else {
        return false;
    };
    if same_network_origin(&locator, &origin) {
        return true;
    }
    if origin.as_str().trim_end_matches('/') != LIVE_BFL_ORIGIN {
        return false;
    }
    let Some(host) = locator.host_str() else {
        return false;
    };
    locator.scheme() == "https"
        && locator.port_or_known_default() == Some(443)
        && (host == "api.bfl.ai" || (host.starts_with("api.") && host.ends_with(".bfl.ai")))
        && locator.path() == "/v1/get_result"
}

fn valid_poll_shape(locator: &str, origin: &str) -> bool {
    if locator.is_empty()
        || locator.len() > 8192
        || locator.contains('#')
        || parse_poll_request_id(locator).is_none()
    {
        return false;
    }
    let Some((endpoint, _)) = locator.split_once('?') else {
        return false;
    };
    if check_endpoint_url(endpoint).is_err() {
        return false;
    }
    let (Ok(parsed), Ok(origin)) = (url::Url::parse(locator), url::Url::parse(origin)) else {
        return false;
    };
    let expected_path = if same_network_origin(&parsed, &origin) {
        parsed
            .path()
            .strip_prefix(origin.path().trim_end_matches('/'))
    } else {
        Some(parsed.path())
    };
    let Some(query) = parsed.query() else {
        return false;
    };
    if query.contains(['\\', '\0', '#', '@', '|', '%', '\"', '<', '>', '`']) {
        return false;
    }
    expected_path == Some("/v1/get_result")
}

fn valid_request_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 256
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn valid_credit_receipt(value: &str) -> bool {
    if value.is_empty() || value.len() > 32 || value.starts_with('-') || value.starts_with('+') {
        return false;
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_digit() || byte == b'.')
    {
        return false;
    }
    let mut dots = 0;
    let mut digits = 0;
    for byte in value.bytes() {
        if byte == b'.' {
            dots += 1;
        } else {
            digits += 1;
        }
    }
    dots <= 1 && digits >= 1 && !value.starts_with('.') && !value.ends_with('.')
}

fn parse_poll_request_id(polling_url: &str) -> Option<&str> {
    let (_, query) = polling_url.split_once('?')?;
    let mut request_id = None;
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=')?;
        if key == "id" {
            if request_id.is_some() || !valid_request_id(value) {
                return None;
            }
            request_id = Some(value);
        }
    }
    request_id
}

fn poll_id_matches(polling_url: &str, request_id: &str) -> bool {
    parse_poll_request_id(polling_url).is_some_and(|id| id == request_id)
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
        let (bytes, EncodedMediaFacts::Png { width, height }) = inspect_png_off_worker(bytes)
            .await
            .map_err(|_| ApiError::Output)?
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
    if let Some(address) = fixture {
        if !locator.starts_with(&format!("http://{address}/")) {
            return Err((NativeHttpRefusal::InvalidEndpoint, "url_contract"));
        }
    } else if delivery_host_of(locator).is_none() {
        return Err((NativeHttpRefusal::InvalidEndpoint, "url_contract"));
    }
    let (bytes, mime, observed) =
        df_providers::fetch_bfl_delivery(http, locator, max_bytes, remaining, fixture)
            .await
            .map_err(|failure: df_providers::BflDeliveryFailure| {
                if let Some(status) = failure.http_status {
                    *http_status = Some(status);
                }
                (failure.refusal, failure.reason)
            })?;
    if let Some(status) = observed {
        *http_status = Some(status);
    }
    Ok((bytes, mime))
}

fn json_body(response: &df_providers::NativeHttpResponse) -> Result<Value, ApiError> {
    if response.body.len() > 64 * 1024 {
        return Err(ApiError::Output);
    }
    let value = parse_unique_json(
        &response.body,
        TextSchemaLimits {
            maximum_schema_bytes: 64 * 1024,
            maximum_depth: 16,
            maximum_nodes: 2048,
        },
    )
    .map_err(|_| ApiError::Output)?;
    if !value.is_object() {
        return Err(ApiError::Output);
    }
    Ok(value)
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

#[cfg(test)]
mod origin_tests {
    use super::*;

    #[test]
    fn canonical_network_origins_preserve_live_policy_and_exact_base_path() {
        for origin in [
            "HTTPS://API.BFL.AI",
            "https://Api.Bfl.Ai:443/",
            LIVE_BFL_ORIGIN,
        ] {
            let config = BflConfig::operator(origin).expect("config");
            assert_eq!(config.origin, LIVE_BFL_ORIGIN);
            assert!(poll_allowed(
                "https://api.bfl.ai:443/v1/get_result?id=x",
                &config.origin
            ));
            assert!(poll_allowed(
                "HTTPS://API.REGIONAL.BFL.AI/v1/get_result?id=x",
                &config.origin
            ));
            for locator in [
                "https://api.bfl.ai:444/v1/get_result?id=x",
                "https://api.bfl.ai.evil.test/v1/get_result?id=x",
                "https://api.bfl.ai@evil.test/v1/get_result?id=x",
                "http://api.bfl.ai/v1/get_result?id=x",
            ] {
                assert!(!poll_allowed(locator, &config.origin), "{locator}");
            }
        }
        let origin = "HTTPS://API.BFL.AI:443/team%20one";
        let config = BflConfig::operator(origin).expect("config");
        assert!(poll_allowed(
            "https://api.bfl.ai/team%20one/v1/get_result?id=x",
            &config.origin
        ));
        for locator in [
            "https://api.bfl.ai/team%20one-other/v1/get_result?id=x",
            "https://api.bfl.ai/team%20One/v1/get_result?id=x",
            "https://api.bfl.ai:444/team%20one/v1/get_result?id=x",
            "https://api.bfl.ai/v1/get_result?id=x",
        ] {
            assert!(!poll_allowed(locator, &config.origin), "{locator}");
        }
    }
}
