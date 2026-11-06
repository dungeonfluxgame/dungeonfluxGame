//! Native World Labs Marble client: generate Gaussian-splat worlds from one image.
//!
//! Flow (see `planning/worldlabs-splat-worlds-research.md`): one
//! `POST /marble/v1/worlds:generate`, then `GET /marble/v1/operations/{id}` polls,
//! then a free synchronous `POST /marble/v1/worlds/{id}:export` for a PLY signed
//! URL, then plain credential-free downloads of the PLY / collider bytes.
//! Splat files are PLY only here; SPZ URLs in a world response are never read.
//! The adapter never retries and never logs or Debug-prints the API key, the
//! image, or the prompt. Spend is recorded by the caller from
//! [`WorldResult::credits`]; this client records, never blocks, on cost.
//!
//! The shared `NativeHttp` transport cannot carry this provider: it only builds
//! Bearer / fal / ElevenLabs credential headers (Marble needs `WLT-Api-Key`) and
//! it accepts only JSON or MPEG bodies, while a PLY is binary. This module
//! therefore builds its own `reqwest` client from the shared hardened posture
//! (no redirects, no proxy, no retries, bounded connect / read / total timeouts,
//! bounded headers and streamed bounded bodies) and validates its bounds through
//! `NativeHttp` construction.

use std::{
    fmt,
    net::{IpAddr, SocketAddr},
    time::Duration,
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};

use crate::native_http::{
    ExchangeRecord, ExchangeTrace, MAXIMUM_CHUNKS, NativeHttp, NativeHttpCause, SendState,
    exchange_host, hardened_client_builder, header_block_bytes, is_loopback_socket,
    is_valid_credential,
};
use crate::output_fetch::{MAXIMUM_LOCATOR_BYTES, is_public};
use crate::{HttpMethod, ProviderLimits, ProviderName};

const LIVE_BASE: &str = "https://api.worldlabs.ai";
// The image, display name and prompt bounds are the Marble request contract.
const MAXIMUM_IMAGE_BYTES: usize = 8 * 1024 * 1024;
const MAXIMUM_DISPLAY_NAME_CHARACTERS: usize = 64;
const MAXIMUM_TEXT_PROMPT_CHARACTERS: usize = 4_000;
/// The largest request body sent, in bytes: a base64 image of [`MAXIMUM_IMAGE_BYTES`] and JSON.
const MAXIMUM_REQUEST_BYTES: usize = 16 * 1024 * 1024;
/// The largest JSON answer kept, in bytes (operations are a few KiB).
const MAXIMUM_JSON_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
/// The largest response header block, in bytes.
const MAXIMUM_HEADER_BYTES: usize = 16 * 1024;
/// The largest download a caller may ask for, in bytes: room for full-resolution PLY splats.
const MAXIMUM_DOWNLOAD_BYTES: u64 = 1024 * 1024 * 1024;
/// The longest operation failure message kept, in characters.
const MAXIMUM_FAILURE_MESSAGE_CHARACTERS: usize = 512;
const EXPORT_POLL_INTERVAL: Duration = Duration::from_millis(250);
const DOWNLOAD_HOSTS: &[&str] = &["cdn.marble.worldlabs.ai", "wlt-ai-cdn.art"];

/// The Marble models this client can ask for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MarbleModel {
    Draft,
    Marble11,
}

impl MarbleModel {
    /// The published model name, sent as `model` in a generate request.
    pub const fn key(self) -> &'static str {
        match self {
            Self::Draft => "marble-1.0-draft",
            Self::Marble11 => "marble-1.1",
        }
    }

    /// World-step credits (research section 3); the pano step adds 80 for a
    /// non-pano image. For the caller's spend ledger, never a cap.
    pub const fn world_credits(self) -> u64 {
        match self {
            Self::Draft => 150,
            Self::Marble11 => 1_500,
        }
    }
}

impl fmt::Display for MarbleModel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.key())
    }
}

/// PLY export resolutions (`resolution` in the export request).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlyResolution {
    K100,
    K500,
    FullRes,
}

impl PlyResolution {
    /// The `resolution` value of an export request.
    pub const fn key(self) -> &'static str {
        match self {
            Self::K100 => "100k",
            Self::K500 => "500k",
            Self::FullRes => "full_res",
        }
    }
}

/// A World Labs operation identifier returned by [`NativeWorldLabs::submit`].
#[derive(Clone, Eq, Hash, PartialEq)]
pub struct OperationId(String);

impl OperationId {
    pub fn new(value: &str) -> Result<Self, WorldLabsError> {
        if !valid_id(value) {
            return Err(WorldLabsError::InvalidRequest);
        }
        Ok(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for OperationId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("OperationId").field(&self.0).finish()
    }
}

impl fmt::Display for OperationId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

/// One world generation request: a JPEG, its display name and text guidance.
#[derive(Clone, Copy)]
pub struct WorldRequest<'a> {
    pub display_name: &'a str,
    pub image_jpeg: &'a [u8],
    pub text_prompt: &'a str,
    pub model: MarbleModel,
}

impl fmt::Debug for WorldRequest<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WorldRequest")
            .field("display_name", &self.display_name)
            .field("image_jpeg", &self.image_jpeg.len())
            .field("text_prompt", &self.text_prompt.len())
            .field("model", &self.model)
            .finish()
    }
}

/// A completed world, parsed from the operation's `response`.
#[derive(Clone, Debug, PartialEq)]
pub struct WorldResult {
    pub world_id: String,
    pub collider_url: Option<String>,
    pub thumbnail_url: Option<String>,
    pub caption: String,
    pub metric_scale_factor: Option<f64>,
    pub ground_plane_offset: Option<f64>,
    pub credits: Option<u64>,
}

/// One poll of an operation.
#[derive(Clone, Debug, PartialEq)]
pub enum WorldPoll {
    Pending,
    Done(WorldResult),
    Failed(String),
}

/// Why a World Labs call produced no result. Never carries the key, image,
/// prompt, or a signed URL.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorldLabsError {
    InvalidRequest,
    Unauthorized,
    InsufficientCredits,
    RateLimited,
    ContentPolicy,
    Timeout,
    /// The API answered this other HTTP status.
    Provider(u16),
    Output,
    /// The exchange failed or broke a transport bound; the cause says which.
    Transport(NativeHttpCause),
}

impl WorldLabsError {
    pub const fn key(self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid_request",
            Self::Unauthorized => "unauthorized",
            Self::InsufficientCredits => "insufficient_credits",
            Self::RateLimited => "rate_limited",
            Self::ContentPolicy => "content_policy",
            Self::Timeout => "timeout",
            Self::Provider(_) => "http_status",
            Self::Output => "bad_output",
            Self::Transport(_) => "transport",
        }
    }

    pub const fn is_transient(self) -> bool {
        matches!(
            self,
            Self::RateLimited | Self::Timeout | Self::Transport(_) | Self::Provider(500..=599)
        )
    }
}

impl fmt::Display for WorldLabsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.key())
    }
}

impl std::error::Error for WorldLabsError {}

/// Where the adapter sends calls.
pub struct WorldLabsConfig {
    fixture: Option<SocketAddr>,
}

impl WorldLabsConfig {
    pub fn live() -> Self {
        Self { fixture: None }
    }

    /// A literal loopback socket for tests.
    pub fn loopback_fixture(address: SocketAddr) -> Result<Self, WorldLabsError> {
        if !is_loopback_socket(address) {
            return Err(WorldLabsError::InvalidRequest);
        }
        Ok(Self {
            fixture: Some(address),
        })
    }
}

pub struct NativeWorldLabs {
    base: String,
    fixture: Option<SocketAddr>,
    api_key: String,
    total_timeout: Duration,
    client: reqwest::Client,
}

impl fmt::Debug for NativeWorldLabs {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeWorldLabs")
            .finish_non_exhaustive()
    }
}

impl NativeWorldLabs {
    /// `total_timeout` bounds each individual HTTP exchange (one submit, one
    /// poll, one export step, one download).
    pub fn new(
        config: WorldLabsConfig,
        api_key: &str,
        total_timeout: Duration,
    ) -> Result<Self, WorldLabsError> {
        if !is_valid_credential(api_key) {
            return Err(WorldLabsError::InvalidRequest);
        }
        let limits = ProviderLimits::new(
            total_timeout,
            MAXIMUM_REQUEST_BYTES,
            MAXIMUM_JSON_RESPONSE_BYTES,
        )
        .header_bytes(MAXIMUM_HEADER_BYTES)
        .build();
        // Only the validation is wanted: the shared transport refuses a zero or over-long timeout.
        NativeHttp::new(limits, config.fixture.is_some())
            .map_err(|_| WorldLabsError::InvalidRequest)?;
        let client = hardened_client_builder(&limits, config.fixture.is_none())
            .build()
            .map_err(|_| WorldLabsError::InvalidRequest)?;
        Ok(Self {
            base: config.fixture.map_or_else(
                || LIVE_BASE.to_owned(),
                |address| format!("http://{address}"),
            ),
            fixture: config.fixture,
            api_key: api_key.to_owned(),
            total_timeout,
            client,
        })
    }

    /// Submits one image-to-world generation. Returns at once with the
    /// operation to poll; never waits for generation itself.
    pub async fn submit(&self, request: &WorldRequest<'_>) -> Result<OperationId, WorldLabsError> {
        let body = submit_body(request)?;
        let value = self
            .api_json(
                HttpMethod::Post,
                "generate",
                "/marble/v1/worlds:generate",
                Some(body),
            )
            .await?;
        let id = value
            .get("operation_id")
            .and_then(Value::as_str)
            .ok_or(WorldLabsError::Output)?;
        OperationId::new(id).map_err(|_| WorldLabsError::Output)
    }

    /// One `GET` of an operation: pending, done with its world, or failed.
    pub async fn poll(&self, operation: &OperationId) -> Result<WorldPoll, WorldLabsError> {
        let value = self
            .api_json(
                HttpMethod::Get,
                "poll",
                &format!("/marble/v1/operations/{}", operation.as_str()),
                None,
            )
            .await?;
        parse_poll(&value)
    }

    /// Exports splats as PLY at `resolution` and returns the signed URL.
    /// PLY export is synchronous and free; if the reply names an unfinished
    /// operation, that operation is polled (bounded by the total timeout per
    /// exchange) until its `response.url` appears.
    pub async fn export_ply(
        &self,
        world_id: &str,
        resolution: PlyResolution,
    ) -> Result<String, WorldLabsError> {
        if !valid_id(world_id) {
            return Err(WorldLabsError::InvalidRequest);
        }
        let body = json!({
            "asset_type": "splats",
            "format": "ply",
            "resolution": resolution.key(),
        });
        let mut value = self
            .api_json(
                HttpMethod::Post,
                "export",
                &format!("/marble/v1/worlds/{world_id}:export"),
                Some(body.to_string().into_bytes()),
            )
            .await?;
        let started = std::time::Instant::now();
        loop {
            if let Some(url) = export_url(&value) {
                return Ok(url);
            }
            if value.get("done").and_then(Value::as_bool) == Some(true) {
                if operation_error(&value).is_some() {
                    return Err(WorldLabsError::Output);
                }
                // A finished export that carries an operation id may still
                // resolve through one read of that operation.
                if value.get("response").is_some_and(|r| !r.is_null()) {
                    return Err(WorldLabsError::Output);
                }
            }
            let Some(next) = value
                .get("operation_id")
                .and_then(Value::as_str)
                .filter(|id| valid_id(id))
            else {
                return Err(WorldLabsError::Output);
            };
            // Do not re-read the operation we have just been handed in a
            // tight loop forever; the overall wait is bounded.
            if started.elapsed() > self.total_timeout.saturating_mul(4) {
                return Err(WorldLabsError::Timeout);
            }
            tokio::time::sleep(EXPORT_POLL_INTERVAL).await;
            value = self
                .api_json(
                    HttpMethod::Get,
                    "poll",
                    &format!("/marble/v1/operations/{next}"),
                    None,
                )
                .await?;
        }
    }

    /// Downloads `url` (a signed asset URL) with no key header, keeping at
    /// most `maximum_bytes`. Exceeding the bound is an error, never a
    /// truncated success.
    /// Writes one `PROVIDER exchange operation=download` line naming only the download host.
    pub async fn download(&self, url: &str, maximum_bytes: u64) -> Result<Vec<u8>, WorldLabsError> {
        let started = std::time::Instant::now();
        let mut trace = ExchangeTrace::default();
        let result = self.download_traced(url, maximum_bytes, &mut trace).await;
        let host = self
            .resolve_download_url(url)
            .map_or_else(|_| "-".to_owned(), |target| exchange_host(&target));
        ExchangeRecord {
            provider: ProviderName::WorldLabs,
            operation: Some("download"),
            host: &host,
            status: trace.status,
            cause: match &result {
                Ok(_) => "ok",
                Err(WorldLabsError::Transport(cause)) => cause.key(),
                Err(error) => error.key(),
            },
            sent: trace.sent,
            request_bytes: 0,
            response_bytes: result.as_ref().map_or(0, Vec::len),
            duration: started.elapsed(),
            request_id: None,
        }
        .record();
        result
    }

    async fn download_traced(
        &self,
        url: &str,
        maximum_bytes: u64,
        trace: &mut ExchangeTrace,
    ) -> Result<Vec<u8>, WorldLabsError> {
        if maximum_bytes == 0 || maximum_bytes > MAXIMUM_DOWNLOAD_BYTES {
            return Err(WorldLabsError::InvalidRequest);
        }
        let deadline = tokio::time::Instant::now() + self.total_timeout;
        let target = self.resolve_download_url(url)?;
        let (client, pinned_addresses) = match self.fixture {
            Some(_) => (self.client.clone(), None),
            None => {
                let (client, addresses) =
                    pinned_download_client(&target, self.total_timeout, deadline).await?;
                (client, Some(addresses))
            }
        };
        let send = client.get(&target).send();
        // From here the supplier may have the request, whatever happens next.
        trace.sent = SendState::PossiblySent;
        let mut response = tokio::time::timeout_at(deadline, send)
            .await
            .map_err(|_| WorldLabsError::Timeout)?
            .map_err(map_reqwest_error)?;
        trace.sent = SendState::KnownSent;
        trace.status = Some(response.status().as_u16());
        if let Some(accepted) = &pinned_addresses {
            let connected = response
                .remote_addr()
                .ok_or(WorldLabsError::Transport(NativeHttpCause::Unknown))?;
            if !accepted.contains(&connected.ip()) {
                return Err(WorldLabsError::Transport(NativeHttpCause::InvalidEndpoint));
            }
        }
        if response.status().is_redirection() {
            return Err(WorldLabsError::Transport(NativeHttpCause::Redirect));
        }
        let status = response.status().as_u16();
        if status != 200 {
            return Err(map_status(status, None));
        }
        check_header_bytes(&response)?;
        if response
            .content_length()
            .is_some_and(|length| length > maximum_bytes)
        {
            return Err(WorldLabsError::Transport(NativeHttpCause::ResponseTooLarge));
        }
        let mut body: Vec<u8> = Vec::new();
        let mut chunks = 0usize;
        loop {
            let next = tokio::time::timeout_at(deadline, response.chunk())
                .await
                .map_err(|_| WorldLabsError::Timeout)?;
            let Some(chunk) = next.map_err(map_reqwest_error)? else {
                break;
            };
            chunks += 1;
            if chunks > MAXIMUM_CHUNKS {
                return Err(WorldLabsError::Transport(NativeHttpCause::TooManyChunks));
            }
            let length = (body.len() as u64)
                .checked_add(chunk.len() as u64)
                .ok_or(WorldLabsError::Transport(NativeHttpCause::ResponseTooLarge))?;
            if length > maximum_bytes {
                return Err(WorldLabsError::Transport(NativeHttpCause::ResponseTooLarge));
            }
            body.try_reserve(chunk.len())
                .map_err(|_| WorldLabsError::Transport(NativeHttpCause::ResponseTooLarge))?;
            body.extend_from_slice(&chunk);
        }
        if body.is_empty() {
            return Err(WorldLabsError::Output);
        }
        Ok(body)
    }

    fn resolve_download_url(&self, url: &str) -> Result<String, WorldLabsError> {
        if url.is_empty() || url.len() > MAXIMUM_LOCATOR_BYTES {
            return Err(WorldLabsError::InvalidRequest);
        }
        if let Some(address) = self.fixture {
            if let Some(path) = url.strip_prefix('/') {
                if path.starts_with('/') {
                    return Err(WorldLabsError::InvalidRequest);
                }
                return Ok(format!("http://{address}/{path}"));
            }
            let parsed = reqwest::Url::parse(url).map_err(|_| WorldLabsError::InvalidRequest)?;
            let mut target = format!("http://{address}{}", parsed.path());
            if let Some(query) = parsed.query() {
                target.push('?');
                target.push_str(query);
            }
            return Ok(target);
        }
        let parsed = reqwest::Url::parse(url).map_err(|_| WorldLabsError::InvalidRequest)?;
        if parsed.scheme() != "https"
            || !parsed
                .host_str()
                .is_some_and(|host| DOWNLOAD_HOSTS.contains(&host))
            || parsed.port().is_some_and(|port| port != 443)
            || !parsed.username().is_empty()
            || parsed.password().is_some()
        {
            return Err(WorldLabsError::InvalidRequest);
        }
        Ok(url.to_owned())
    }

    /// One JSON exchange with the API, sent with exactly the given method, and its
    /// `PROVIDER exchange` audit line labelled `operation`.
    async fn api_json(
        &self,
        method: HttpMethod,
        operation: &'static str,
        path: &str,
        body: Option<Vec<u8>>,
    ) -> Result<Value, WorldLabsError> {
        let started = std::time::Instant::now();
        let request_bytes = body.as_ref().map_or(0, Vec::len);
        let mut trace = ExchangeTrace::default();
        let mut response_bytes = 0;
        let result = self
            .api_json_traced(method, path, body, &mut trace, &mut response_bytes)
            .await;
        ExchangeRecord {
            provider: ProviderName::WorldLabs,
            operation: Some(operation),
            host: &exchange_host(&self.base),
            status: trace.status,
            cause: match &result {
                Ok(_) => "ok",
                Err(WorldLabsError::Transport(cause)) => cause.key(),
                Err(error) => error.key(),
            },
            sent: trace.sent,
            request_bytes,
            response_bytes,
            duration: started.elapsed(),
            request_id: None,
        }
        .record();
        result
    }

    async fn api_json_traced(
        &self,
        method: HttpMethod,
        path: &str,
        body: Option<Vec<u8>>,
        trace: &mut ExchangeTrace,
        response_bytes: &mut usize,
    ) -> Result<Value, WorldLabsError> {
        let url = format!("{}{path}", self.base);
        let mut builder = match method {
            HttpMethod::Get => self.client.get(&url),
            HttpMethod::Post => self.client.post(&url),
            HttpMethod::Put => self.client.put(&url),
        }
        .header("WLT-Api-Key", &self.api_key)
        .header("Content-Type", "application/json");
        if let Some(body) = body {
            builder = builder.body(body);
        }
        let send = builder.send();
        trace.sent = SendState::PossiblySent;
        let mut response = tokio::time::timeout(self.total_timeout, send)
            .await
            .map_err(|_| WorldLabsError::Timeout)?
            .map_err(map_reqwest_error)?;
        trace.sent = SendState::KnownSent;
        trace.status = Some(response.status().as_u16());
        if response.status().is_redirection() {
            return Err(WorldLabsError::Transport(NativeHttpCause::Redirect));
        }
        check_header_bytes(&response)?;
        let status = response.status().as_u16();
        let mut bytes: Vec<u8> = Vec::new();
        let mut chunks = 0usize;
        loop {
            let next = tokio::time::timeout(self.total_timeout, response.chunk())
                .await
                .map_err(|_| WorldLabsError::Timeout)?;
            let Some(chunk) = next.map_err(map_reqwest_error)? else {
                break;
            };
            chunks += 1;
            if chunks > MAXIMUM_CHUNKS {
                return Err(WorldLabsError::Transport(NativeHttpCause::TooManyChunks));
            }
            if bytes
                .len()
                .checked_add(chunk.len())
                .is_none_or(|len| len > MAXIMUM_JSON_RESPONSE_BYTES)
            {
                return Err(WorldLabsError::Transport(NativeHttpCause::ResponseTooLarge));
            }
            bytes.extend_from_slice(&chunk);
            *response_bytes = bytes.len();
        }
        if status != 200 {
            return Err(map_status(status, Some(&bytes)));
        }
        let content_ok = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| {
                value
                    .split(';')
                    .next()
                    .is_some_and(|media| media.trim().eq_ignore_ascii_case("application/json"))
            });
        // Loopback fixtures in this repo always send application/json; a live
        // 200 without it is not a Marble reply we can trust.
        if !content_ok {
            return Err(WorldLabsError::Output);
        }
        parse_json(&bytes)
    }
}

async fn pinned_download_client(
    target: &str,
    total_timeout: Duration,
    deadline: tokio::time::Instant,
) -> Result<(reqwest::Client, Vec<IpAddr>), WorldLabsError> {
    let parsed = reqwest::Url::parse(target).map_err(|_| WorldLabsError::InvalidRequest)?;
    let host = parsed.host_str().ok_or(WorldLabsError::InvalidRequest)?;
    let port = parsed
        .port_or_known_default()
        .ok_or(WorldLabsError::InvalidRequest)?;
    let resolved: Vec<SocketAddr> =
        tokio::time::timeout_at(deadline, tokio::net::lookup_host((host, port)))
            .await
            .map_err(|_| WorldLabsError::Timeout)?
            .map_err(|_| WorldLabsError::Transport(NativeHttpCause::Unknown))?
            .collect();
    if resolved.is_empty() || resolved.iter().any(|address| !is_public(address.ip())) {
        return Err(WorldLabsError::Transport(NativeHttpCause::InvalidEndpoint));
    }
    // Only the timeouts shape the client; the download bound is enforced while reading.
    let limits = ProviderLimits::new(
        total_timeout,
        MAXIMUM_REQUEST_BYTES,
        MAXIMUM_JSON_RESPONSE_BYTES,
    )
    .build();
    let client = hardened_client_builder(&limits, true)
        .resolve_to_addrs(host, &resolved)
        .build()
        .map_err(|_| WorldLabsError::Transport(NativeHttpCause::Unknown))?;
    Ok((
        client,
        resolved.into_iter().map(|address| address.ip()).collect(),
    ))
}

fn submit_body(request: &WorldRequest<'_>) -> Result<Vec<u8>, WorldLabsError> {
    if request.display_name.chars().count() > MAXIMUM_DISPLAY_NAME_CHARACTERS
        || request.image_jpeg.is_empty()
        || request.image_jpeg.len() > MAXIMUM_IMAGE_BYTES
        || request.text_prompt.chars().count() > MAXIMUM_TEXT_PROMPT_CHARACTERS
    {
        return Err(WorldLabsError::InvalidRequest);
    }
    let mut world_prompt = json!({
        "type": "image",
        "image_prompt": {
            "source": "data_base64",
            "data_base64": STANDARD.encode(request.image_jpeg),
            "extension": "jpg",
        },
        "is_pano": false,
    });
    if !request.text_prompt.is_empty() {
        world_prompt["text_prompt"] = Value::String(request.text_prompt.to_owned());
    }
    serde_json::to_vec(&json!({
        "display_name": request.display_name,
        "model": request.model.key(),
        "permission": {"public": false},
        "tags": ["dungeonflux-poc"],
        "world_prompt": world_prompt,
    }))
    .map_err(|_| WorldLabsError::InvalidRequest)
}

fn parse_json(bytes: &[u8]) -> Result<Value, WorldLabsError> {
    // Reject duplicate keys at the top level is unnecessary here; the fields
    // read below are each checked for type, and unknown shapes fail closed.
    serde_json::from_slice(bytes).map_err(|_| WorldLabsError::Output)
}

fn check_header_bytes(response: &reqwest::Response) -> Result<(), WorldLabsError> {
    if header_block_bytes(response.headers()).is_none_or(|bytes| bytes > MAXIMUM_HEADER_BYTES) {
        return Err(WorldLabsError::Transport(NativeHttpCause::HeadersTooLarge));
    }
    Ok(())
}

fn map_reqwest_error(error: reqwest::Error) -> WorldLabsError {
    if error.is_timeout() {
        WorldLabsError::Timeout
    } else {
        WorldLabsError::Transport(NativeHttpCause::Unknown)
    }
}

fn map_status(status: u16, body: Option<&[u8]>) -> WorldLabsError {
    match status {
        401 | 403 => WorldLabsError::Unauthorized,
        402 => WorldLabsError::InsufficientCredits,
        429 => WorldLabsError::RateLimited,
        400 => {
            let detail = body
                .and_then(|bytes| serde_json::from_slice::<Value>(bytes).ok())
                .and_then(|value| {
                    value
                        .get("detail")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })
                .unwrap_or_default()
                .to_ascii_lowercase();
            if (detail.contains("content") && detail.contains("polic"))
                || detail.contains("moderation")
            {
                WorldLabsError::ContentPolicy
            } else {
                WorldLabsError::InvalidRequest
            }
        }
        422 => WorldLabsError::InvalidRequest,
        other => WorldLabsError::Provider(other),
    }
}

fn operation_error(value: &Value) -> Option<String> {
    let error = value.get("error")?;
    if error.is_null() {
        return None;
    }
    if let Some(text) = error.as_str() {
        return Some(truncate_failed(text));
    }
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .map(truncate_failed);
    if let Some(message) = message {
        return Some(message);
    }
    if let Some(code) = error.get("code") {
        let code = match code {
            Value::Number(number) => number.to_string(),
            Value::String(text) => truncate_failed(text),
            _ => return Some("operation failed".to_owned()),
        };
        return Some(format!("operation failed (code {code})"));
    }
    Some("operation failed".to_owned())
}

fn truncate_failed(text: &str) -> String {
    text.chars()
        .take(MAXIMUM_FAILURE_MESSAGE_CHARACTERS)
        .collect()
}

fn export_url(value: &Value) -> Option<String> {
    value
        .get("response")
        .and_then(|response| response.get("url"))
        .or_else(|| value.get("url"))
        .and_then(Value::as_str)
        .filter(|url| !url.is_empty() && url.len() <= MAXIMUM_LOCATOR_BYTES)
        .map(str::to_owned)
}

/// The credits a cost figure records: a whole number as is, a fractional one rounded up so spend
/// is never under-recorded. A negative, non-finite or out-of-range figure records nothing rather
/// than a fabricated zero.
fn spent_credits(credits: &Value) -> Option<u64> {
    if let Some(whole) = credits.as_u64() {
        return Some(whole);
    }
    let figure = credits.as_f64()?.ceil();
    // `u64::MAX as f64` rounds up to 2^64, so the strict bound keeps the conversion exact.
    (figure.is_finite() && figure >= 0.0 && figure < u64::MAX as f64).then_some(figure as u64)
}

fn parse_poll(value: &Value) -> Result<WorldPoll, WorldLabsError> {
    let done = value
        .get("done")
        .and_then(Value::as_bool)
        .ok_or(WorldLabsError::Output)?;
    if !done {
        return Ok(WorldPoll::Pending);
    }
    if let Some(message) = operation_error(value) {
        return Ok(WorldPoll::Failed(message));
    }
    let response = value
        .get("response")
        .filter(|response| !response.is_null())
        .ok_or(WorldLabsError::Output)?;
    let world_id = response
        .get("world_id")
        .and_then(Value::as_str)
        .filter(|id| valid_id(id))
        .ok_or(WorldLabsError::Output)?
        .to_owned();
    let assets = response.get("assets");
    let collider_url = assets
        .and_then(|assets| assets.get("mesh"))
        .and_then(|mesh| mesh.get("collider_mesh_url"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let thumbnail_url = assets
        .and_then(|assets| assets.get("thumbnail_url"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let caption = assets
        .and_then(|assets| assets.get("caption"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    // Saved operation JSON (m11 and draft): the values live at
    // response.assets.splats.semantics_metadata.{metric_scale_factor,ground_plane_offset};
    // draft has semantics_metadata: null, hence both stay None there.
    let semantics = assets
        .and_then(|assets| assets.get("splats"))
        .and_then(|splats| splats.get("semantics_metadata"))
        .filter(|meta| !meta.is_null());
    let finite = |meta: Option<&Value>, key: &str| {
        meta.and_then(|meta| meta.get(key))
            .and_then(Value::as_f64)
            .filter(|number| number.is_finite())
    };
    let credits = value
        .get("cost")
        .and_then(|cost| cost.get("total_credits"))
        .and_then(spent_credits);
    Ok(WorldPoll::Done(WorldResult {
        world_id,
        collider_url,
        thumbnail_url,
        caption,
        metric_scale_factor: finite(semantics, "metric_scale_factor"),
        ground_plane_offset: finite(semantics, "ground_plane_offset"),
        credits,
    }))
}

#[cfg(test)]
mod worldlabs_tests;
