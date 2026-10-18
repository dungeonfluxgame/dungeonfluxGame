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
//! it caps responses at 16 MiB with a JSON content type, while a 500k PLY is
//! 28 MB of binary. This module therefore builds its own `reqwest` client with
//! the same posture (no redirects, no proxy, no retries, bounded connect / read
//! / total timeouts, bounded headers and streamed bounded bodies) and validates
//! its bounds through [`NativeHttpLimits`] / `NativeHttp` construction.

use std::{fmt, net::SocketAddr, time::Duration};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};

use crate::NativeHttpLimits;
use crate::native_http::NativeHttp;

const LIVE_BASE: &str = "https://api.worldlabs.ai";
const MAXIMUM_IMAGE_BYTES: usize = 8 * 1024 * 1024;
const MAXIMUM_DISPLAY_NAME_CHARACTERS: usize = 64;
const MAXIMUM_TEXT_PROMPT_CHARACTERS: usize = 4_000;
const MAXIMUM_JSON_RESPONSE_BYTES: usize = 1024 * 1024;
const MAXIMUM_HEADER_BYTES: usize = 16 * 1024;
const MAXIMUM_CHUNKS: usize = 65_536;
const MAXIMUM_DOWNLOAD_BYTES: u64 = 256 * 1024 * 1024;
const EXPORT_POLL_INTERVAL: Duration = Duration::from_millis(250);

/// The Marble models this client can ask for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MarbleModel {
    Draft,
    Marble11,
}

impl MarbleModel {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "marble-1.0-draft",
            Self::Marble11 => "marble-1.1",
        }
    }

    pub const fn label(self) -> &'static str {
        self.as_str()
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
        formatter.write_str(self.as_str())
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
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::K100 => "100k",
            Self::K500 => "500k",
            Self::FullRes => "full_res",
        }
    }

    pub const fn label(self) -> &'static str {
        self.as_str()
    }
}

/// A World Labs operation identifier returned by [`NativeWorldLabs::submit`].
#[derive(Clone, Eq, Hash, PartialEq)]
pub struct OperationId(String);

impl OperationId {
    pub fn new(value: impl Into<String>) -> Result<Self, WorldLabsError> {
        let value = value.into();
        if !valid_id(&value) {
            return Err(WorldLabsError::InvalidRequest);
        }
        Ok(Self(value))
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
    Http(u16),
    Output,
    Transport,
}

impl WorldLabsError {
    pub const fn label(self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid_request",
            Self::Unauthorized => "unauthorized",
            Self::InsufficientCredits => "insufficient_credits",
            Self::RateLimited => "rate_limited",
            Self::ContentPolicy => "content_policy",
            Self::Timeout => "timeout",
            Self::Http(_) => "http_status",
            Self::Output => "bad_output",
            Self::Transport => "transport",
        }
    }

    pub const fn is_transient(self) -> bool {
        matches!(
            self,
            Self::RateLimited | Self::Timeout | Self::Transport | Self::Http(500..=599)
        )
    }
}

impl fmt::Display for WorldLabsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.label())
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
        if !address.ip().is_loopback() || address.port() == 0 {
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
        if api_key.is_empty()
            || api_key.len() > 4096
            || !api_key.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
            || total_timeout.is_zero()
            || total_timeout > Duration::from_secs(180)
        {
            return Err(WorldLabsError::InvalidRequest);
        }
        // Validate the bounds through the shared transport's own gate.
        let limits = NativeHttpLimits {
            connect_timeout: Duration::from_secs(5).min(total_timeout),
            read_timeout: total_timeout,
            total_timeout,
            maximum_request_bytes: 16 * 1024 * 1024,
            maximum_response_bytes: MAXIMUM_JSON_RESPONSE_BYTES,
            maximum_header_bytes: MAXIMUM_HEADER_BYTES,
            maximum_chunks: MAXIMUM_CHUNKS,
        };
        let _ = NativeHttp::new(limits, config.fixture.is_some())
            .map_err(|_| WorldLabsError::InvalidRequest)?;
        let client = reqwest::Client::builder()
            .tls_backend_rustls()
            .https_only(config.fixture.is_none())
            .retry(reqwest::retry::never())
            .redirect(reqwest::redirect::Policy::none())
            .referer(false)
            .connection_verbose(false)
            .no_proxy()
            .connect_timeout(limits.connect_timeout)
            .read_timeout(limits.read_timeout)
            .timeout(limits.total_timeout)
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
            .api_json("POST", "/marble/v1/worlds:generate", Some(body))
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
                "GET",
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
            "resolution": resolution.as_str(),
        });
        let mut value = self
            .api_json(
                "POST",
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
                .api_json("GET", &format!("/marble/v1/operations/{next}"), None)
                .await?;
        }
    }

    /// Downloads `url` (a signed asset URL) with no key header, keeping at
    /// most `maximum_bytes`. Exceeding the bound is an error, never a
    /// truncated success.
    pub async fn download(&self, url: &str, maximum_bytes: u64) -> Result<Vec<u8>, WorldLabsError> {
        if maximum_bytes == 0 || maximum_bytes > MAXIMUM_DOWNLOAD_BYTES {
            return Err(WorldLabsError::InvalidRequest);
        }
        let target = self.resolve_download_url(url)?;
        let send = self.client.get(&target).send();
        let mut response = tokio::time::timeout(self.total_timeout, send)
            .await
            .map_err(|_| WorldLabsError::Timeout)?
            .map_err(map_reqwest_error)?;
        if response.status().is_redirection() {
            return Err(WorldLabsError::Transport);
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
            return Err(WorldLabsError::Transport);
        }
        let mut body: Vec<u8> = Vec::new();
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
                return Err(WorldLabsError::Transport);
            }
            let length = (body.len() as u64)
                .checked_add(chunk.len() as u64)
                .ok_or(WorldLabsError::Transport)?;
            if length > maximum_bytes {
                return Err(WorldLabsError::Transport);
            }
            body.try_reserve(chunk.len())
                .map_err(|_| WorldLabsError::Transport)?;
            body.extend_from_slice(&chunk);
        }
        if body.is_empty() {
            return Err(WorldLabsError::Output);
        }
        Ok(body)
    }

    fn resolve_download_url(&self, url: &str) -> Result<String, WorldLabsError> {
        if url.is_empty() || url.len() > 8192 {
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
            || parsed.host_str().is_none()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
        {
            return Err(WorldLabsError::InvalidRequest);
        }
        Ok(url.to_owned())
    }

    async fn api_json(
        &self,
        method: &str,
        path: &str,
        body: Option<Vec<u8>>,
    ) -> Result<Value, WorldLabsError> {
        let url = format!("{}{path}", self.base);
        let mut builder = match method {
            "GET" => self.client.get(&url),
            _ => self.client.post(&url),
        }
        .header("WLT-Api-Key", &self.api_key)
        .header("Content-Type", "application/json");
        if let Some(body) = body {
            builder = builder.body(body);
        }
        let send = builder.send();
        let mut response = tokio::time::timeout(self.total_timeout, send)
            .await
            .map_err(|_| WorldLabsError::Timeout)?
            .map_err(map_reqwest_error)?;
        if response.status().is_redirection() {
            return Err(WorldLabsError::Transport);
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
            if chunks > MAXIMUM_CHUNKS
                || bytes
                    .len()
                    .checked_add(chunk.len())
                    .is_none_or(|len| len > MAXIMUM_JSON_RESPONSE_BYTES)
            {
                return Err(WorldLabsError::Transport);
            }
            bytes.extend_from_slice(&chunk);
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
        "model": request.model.as_str(),
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
    let mut total = 0usize;
    for (name, value) in response.headers() {
        total = total
            .checked_add(name.as_str().len() + value.as_bytes().len())
            .ok_or(WorldLabsError::Transport)?;
    }
    if total > MAXIMUM_HEADER_BYTES {
        return Err(WorldLabsError::Transport);
    }
    Ok(())
}

fn map_reqwest_error(error: reqwest::Error) -> WorldLabsError {
    if error.is_timeout() {
        WorldLabsError::Timeout
    } else {
        WorldLabsError::Transport
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
        other => WorldLabsError::Http(other),
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
    text.chars().take(512).collect()
}

fn export_url(value: &Value) -> Option<String> {
    value
        .get("response")
        .and_then(|response| response.get("url"))
        .or_else(|| value.get("url"))
        .and_then(Value::as_str)
        .filter(|url| !url.is_empty() && url.len() <= 8192)
        .map(str::to_owned)
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
    let credits = value.get("cost").and_then(|cost| {
        cost.get("total_credits").and_then(|credits| {
            credits
                .as_u64()
                .or_else(|| credits.as_f64().map(|n| n as u64))
        })
    });
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
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, Ordering},
        },
        thread,
        time::Instant,
    };

    const KEY: &str = "fixture-secret-key-123";

    #[derive(Clone)]
    struct Reply {
        status: u16,
        content_type: &'static str,
        body: Vec<u8>,
    }

    fn json_reply(status: u16, value: Value) -> Reply {
        Reply {
            status,
            content_type: "application/json",
            body: serde_json::to_vec(&value).unwrap(),
        }
    }

    struct Seen {
        method: String,
        path: String,
        key: Option<String>,
        body: Vec<u8>,
    }

    struct Fixture {
        address: SocketAddr,
        seen: Arc<Mutex<Vec<Seen>>>,
        stop: Arc<AtomicBool>,
        join: Option<thread::JoinHandle<()>>,
    }

    impl Fixture {
        fn new(replies: Vec<Reply>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let address = listener.local_addr().unwrap();
            let seen = Arc::new(Mutex::new(Vec::new()));
            let record = seen.clone();
            let stop = Arc::new(AtomicBool::new(false));
            let stopped = stop.clone();
            let join = thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(15);
                let mut index = 0usize;
                while !stopped.load(Ordering::Acquire) && Instant::now() < deadline {
                    let (mut socket, _) = match listener.accept() {
                        Ok(pair) => pair,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(2));
                            continue;
                        }
                        Err(error) => panic!("fixture accept: {error}"),
                    };
                    socket.set_nonblocking(false).unwrap();
                    socket
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    let mut data = Vec::new();
                    let end = loop {
                        let mut block = [0; 4096];
                        let count = socket.read(&mut block).unwrap();
                        assert!(count > 0);
                        data.extend_from_slice(&block[..count]);
                        if let Some(end) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                            break end + 4;
                        }
                    };
                    let head = String::from_utf8(data[..end].to_vec()).unwrap();
                    let length = head
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|value| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    while data.len() - end < length {
                        let mut block = [0; 4096];
                        let count = socket.read(&mut block).unwrap();
                        assert!(count > 0);
                        data.extend_from_slice(&block[..count]);
                    }
                    let mut first = head.lines().next().unwrap().split_whitespace();
                    let (method, path) = (
                        first.next().unwrap().to_owned(),
                        first.next().unwrap().to_owned(),
                    );
                    let key = head.lines().find_map(|line| {
                        let lower = line.to_ascii_lowercase();
                        lower
                            .strip_prefix("wlt-api-key:")
                            .map(|value| value.trim().to_owned())
                    });
                    record.lock().unwrap().push(Seen {
                        method,
                        path,
                        key,
                        body: data[end..].to_vec(),
                    });
                    let reply = replies[index.min(replies.len() - 1)].clone();
                    index += 1;
                    let header = format!(
                        "HTTP/1.1 {} fixture\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        reply.status,
                        reply.content_type,
                        reply.body.len()
                    );
                    let _ = socket.write_all(header.as_bytes());
                    let _ = socket.write_all(&reply.body);
                }
            });
            Self {
                address,
                seen,
                stop,
                join: Some(join),
            }
        }

        fn client(&self) -> NativeWorldLabs {
            NativeWorldLabs::new(
                WorldLabsConfig::loopback_fixture(self.address).unwrap(),
                KEY,
                Duration::from_secs(5),
            )
            .unwrap()
        }

        fn finish(&mut self) -> Vec<Seen> {
            self.stop.store(true, Ordering::Release);
            if let Some(join) = self.join.take() {
                join.join().unwrap();
            }
            std::mem::take(&mut *self.seen.lock().unwrap())
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Release);
            if let Some(join) = self.join.take() {
                let _ = join.join();
            }
        }
    }

    fn request() -> WorldRequest<'static> {
        WorldRequest {
            display_name: "cloth shop",
            image_jpeg: b"\xff\xd8\xff\xe0jpeg-bytes",
            text_prompt: "An empty period cloth shop, photographic.",
            model: MarbleModel::Marble11,
        }
    }

    fn done_operation() -> Value {
        json!({
            "operation_id": "op-1",
            "done": true,
            "error": null,
            "response": {
                "world_id": "world-1",
                "assets": {
                    "mesh": {"collider_mesh_url": "https://cdn.example/world-1/c.glb"},
                    "splats": {"semantics_metadata": {"metric_scale_factor": 1.5014322, "ground_plane_offset": 1.2088797}},
                    "thumbnail_url": "https://cdn.example/world-1/t.webp",
                    "caption": "A cloth shop."
                }
            },
            "cost": {"total_credits": 1580, "line_items": []}
        })
    }

    #[test]
    fn model_and_resolution_strings_are_the_api_names() {
        assert_eq!(MarbleModel::Draft.as_str(), "marble-1.0-draft");
        assert_eq!(MarbleModel::Marble11.as_str(), "marble-1.1");
        assert_eq!(PlyResolution::K100.as_str(), "100k");
        assert_eq!(PlyResolution::K500.as_str(), "500k");
        assert_eq!(PlyResolution::FullRes.as_str(), "full_res");
        assert_eq!(
            WorldLabsError::InsufficientCredits.label(),
            "insufficient_credits"
        );
        assert_eq!(WorldLabsError::Http(500).label(), "http_status");
    }

    #[test]
    fn config_and_credential_validation() {
        assert!(WorldLabsConfig::loopback_fixture("8.8.8.8:443".parse().unwrap()).is_err());
        assert!(WorldLabsConfig::loopback_fixture("127.0.0.1:0".parse().unwrap()).is_err());
        assert!(NativeWorldLabs::new(WorldLabsConfig::live(), "", Duration::from_secs(5)).is_err());
        assert!(
            NativeWorldLabs::new(WorldLabsConfig::live(), "has space", Duration::from_secs(5))
                .is_err()
        );
        assert!(NativeWorldLabs::new(WorldLabsConfig::live(), KEY, Duration::ZERO).is_err());
    }

    #[test]
    fn the_key_never_appears_in_debug_output() {
        let mut fixture = Fixture::new(vec![json_reply(
            200,
            json!({"operation_id": "op-1", "done": false}),
        )]);
        let client = fixture.client();
        let req = request();
        let debug = format!("{client:?} {req:?}");
        assert!(!debug.contains(KEY), "{debug}");
        fixture.finish();
    }

    #[test]
    fn parse_poll_reads_proof_scale_fields_and_draft_null() {
        match parse_poll(&done_operation()).unwrap() {
            WorldPoll::Done(result) => {
                assert_eq!(result.metric_scale_factor, Some(1.5014322));
                assert_eq!(result.ground_plane_offset, Some(1.2088797));
                assert_eq!(result.credits, Some(1580));
                assert_eq!(
                    result.collider_url.as_deref(),
                    Some("https://cdn.example/world-1/c.glb")
                );
            }
            other => panic!("{other:?}"),
        }
        let draft = json!({
            "done": true, "error": null,
            "response": {"world_id": "w-draft", "assets": {"splats": {"semantics_metadata": null}, "caption": "x"}},
            "cost": {"total_credits": 230}
        });
        match parse_poll(&draft).unwrap() {
            WorldPoll::Done(result) => {
                assert_eq!(result.metric_scale_factor, None);
                assert_eq!(result.ground_plane_offset, None);
            }
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn submit_sends_image_world_prompt_and_key_header() {
        let mut fixture = Fixture::new(vec![json_reply(
            200,
            json!({"operation_id": "op-abc", "done": false, "error": null}),
        )]);
        let id = fixture.client().submit(&request()).await.unwrap();
        assert_eq!(id.as_str(), "op-abc");
        let seen = fixture.finish();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].method, "POST");
        assert_eq!(seen[0].path, "/marble/v1/worlds:generate");
        assert_eq!(seen[0].key.as_deref(), Some(KEY));
        let sent: Value = serde_json::from_slice(&seen[0].body).unwrap();
        assert_eq!(sent["model"], "marble-1.1");
        assert_eq!(sent["display_name"], "cloth shop");
        assert_eq!(sent["permission"]["public"], false);
        assert_eq!(sent["world_prompt"]["type"], "image");
        assert_eq!(sent["world_prompt"]["is_pano"], false);
        assert_eq!(
            sent["world_prompt"]["image_prompt"]["source"],
            "data_base64"
        );
        assert_eq!(sent["world_prompt"]["image_prompt"]["extension"], "jpg");
        assert!(
            sent["world_prompt"]["image_prompt"]["data_base64"]
                .as_str()
                .unwrap()
                .len()
                > 8
        );
    }

    #[tokio::test]
    async fn poll_pending_then_done_parses_world() {
        let mut fixture = Fixture::new(vec![
            json_reply(
                200,
                json!({"operation_id": "op-1", "done": false, "error": null}),
            ),
            json_reply(200, done_operation()),
        ]);
        let client = fixture.client();
        let op = OperationId::new("op-1").unwrap();
        assert_eq!(client.poll(&op).await.unwrap(), WorldPoll::Pending);
        match client.poll(&op).await.unwrap() {
            WorldPoll::Done(result) => {
                assert_eq!(result.world_id, "world-1");
                assert_eq!(result.caption, "A cloth shop.");
                assert_eq!(
                    result.thumbnail_url.as_deref(),
                    Some("https://cdn.example/world-1/t.webp")
                );
                assert_eq!(result.metric_scale_factor, Some(1.5014322));
                assert_eq!(result.ground_plane_offset, Some(1.2088797));
                assert_eq!(result.credits, Some(1580));
            }
            other => panic!("{other:?}"),
        }
        let seen = fixture.finish();
        assert!(
            seen.iter()
                .all(|call| call.method == "GET" && call.path == "/marble/v1/operations/op-1")
        );
    }

    #[tokio::test]
    async fn poll_failed_operation_returns_message() {
        let mut fixture = Fixture::new(vec![json_reply(
            200,
            json!({"operation_id": "op-1", "done": true, "error": {"code": 400, "message": "generation failed"}}),
        )]);
        let op = OperationId::new("op-1").unwrap();
        assert_eq!(
            fixture.client().poll(&op).await.unwrap(),
            WorldPoll::Failed("generation failed".to_owned())
        );
        fixture.finish();
    }

    #[tokio::test]
    async fn export_ply_returns_signed_url_and_sends_ply_request() {
        let mut fixture = Fixture::new(vec![json_reply(
            200,
            json!({"operation_id": "exp-1", "done": true, "error": null, "response": {"asset_type": "splats", "format": "ply", "resolution": "500k", "url": "https://cdn.example/world-1/x.ply?sig=1"}}),
        )]);
        let url = fixture
            .client()
            .export_ply("world-1", PlyResolution::K500)
            .await
            .unwrap();
        assert_eq!(url, "https://cdn.example/world-1/x.ply?sig=1");
        let seen = fixture.finish();
        assert_eq!(seen[0].path, "/marble/v1/worlds/world-1:export");
        let sent: Value = serde_json::from_slice(&seen[0].body).unwrap();
        assert_eq!(
            sent,
            json!({"asset_type": "splats", "format": "ply", "resolution": "500k"})
        );
    }

    #[tokio::test]
    async fn download_is_bounded_and_sends_no_key() {
        let mut fixture = Fixture::new(vec![
            Reply {
                status: 200,
                content_type: "application/octet-stream",
                body: b"PLYDATA".to_vec(),
            },
            Reply {
                status: 200,
                content_type: "application/octet-stream",
                body: b"PLYDATA".to_vec(),
            },
        ]);
        let client = fixture.client();
        let url = format!("http://{}/asset.ply", fixture.address);
        assert_eq!(client.download(&url, 1024).await.unwrap(), b"PLYDATA");
        assert_eq!(
            client.download(&url, 4).await.unwrap_err(),
            WorldLabsError::Transport
        );
        let seen = fixture.finish();
        assert!(
            seen.iter().all(|call| call.key.is_none()),
            "download must not send the key"
        );
    }

    #[tokio::test]
    async fn status_mapping_402_429_401() {
        for (status, expected) in [
            (402, WorldLabsError::InsufficientCredits),
            (429, WorldLabsError::RateLimited),
            (401, WorldLabsError::Unauthorized),
        ] {
            let mut fixture = Fixture::new(vec![json_reply(status, json!({"detail": "x"}))]);
            assert_eq!(
                fixture.client().submit(&request()).await.unwrap_err(),
                expected,
                "status {status}"
            );
            fixture.finish();
        }
        // 400 with a content-policy detail maps to ContentPolicy.
        let mut fixture = Fixture::new(vec![json_reply(
            400,
            json!({"detail": "Input violates content policy"}),
        )]);
        assert_eq!(
            fixture.client().submit(&request()).await.unwrap_err(),
            WorldLabsError::ContentPolicy
        );
        fixture.finish();
    }

    #[tokio::test]
    async fn invalid_submit_sends_nothing() {
        let mut fixture = Fixture::new(vec![json_reply(200, json!({"operation_id": "op-1"}))]);
        let bad = WorldRequest {
            image_jpeg: b"",
            ..request()
        };
        assert_eq!(
            fixture.client().submit(&bad).await.unwrap_err(),
            WorldLabsError::InvalidRequest
        );
        assert!(fixture.finish().is_empty());
    }
}
