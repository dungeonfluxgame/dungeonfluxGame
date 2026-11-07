//! Owned native HTTPS execution. No redirects, proxies, resends, or detached tasks.

use std::{
    future::Future,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    time::{Duration, Instant, SystemTime},
};

use df_types::saturating_millis;

use crate::{HttpMethod, HttpRequest, ProviderFailureClass, ProviderName, ProviderRequestId};

/// The longest any one exchange (connect, read or total) may be allowed to take: an hour, so a slow
/// provider call during development is bounded by its caller's deadline, not by the transport.
pub const MAXIMUM_EXCHANGE_TIME: Duration = Duration::from_secs(3_600);
/// The largest request or retained response body, in bytes.
pub const MAXIMUM_BODY_BYTES: usize = 256 * 1024 * 1024;
/// The largest response header block, in bytes (a protocol-level bound).
pub const MAXIMUM_HEADER_BYTES: usize = 65_536;
/// The most body chunks one response may arrive in.
pub const MAXIMUM_CHUNKS: usize = 1_048_576;
/// The longest credential any adapter accepts, in bytes. Credentials are header values, so they
/// are also printable ASCII without spaces (see [`is_valid_credential`]).
pub(crate) const MAXIMUM_CREDENTIAL_BYTES: usize = 8192;
/// The longest supplier request or trace id kept from a response header, in bytes.
const MAXIMUM_SUPPLIER_ID_BYTES: usize = 256;
/// Safety kill switch (`on` or `off`): refuses every OpenAI API call before it is sent. Read here,
/// not by the binary, so no caller can build a transport that skips it.
pub const DISABLE_OPENAI_API_VARIABLE: &str = "DF_GAMEPLAY_DISABLE_OPENAI_API";
/// Codex mode (`on` or `off`): when on, OpenAI API calls are refused like the kill switch above.
pub const CODEX_MODE_VARIABLE: &str = "DF_GAMEPLAY_CODEX_MODE";

/// Whether `credential` can travel as a header value: 1 to [`MAXIMUM_CREDENTIAL_BYTES`] bytes of
/// printable ASCII with no space. Every native adapter constructor gates its credential here;
/// the target-neutral request builders in `http_contract` check only that the key is non-empty.
/// Imagery gates its keys here too, so one rule decides what a key may be.
pub fn is_valid_credential(credential: &str) -> bool {
    !credential.is_empty()
        && credential.len() <= MAXIMUM_CREDENTIAL_BYTES
        && credential.bytes().all(is_visible_ascii)
}

/// Printable ASCII other than space (`0x21..=0x7e`): the alphabet of credentials and of the
/// supplier ids this crate keeps.
fn is_visible_ascii(byte: u8) -> bool {
    (0x21..=0x7e).contains(&byte)
}

/// Whether `address` is a usable loopback test socket: any loopback IP and a nonzero port.
pub(crate) fn is_loopback_socket(address: SocketAddr) -> bool {
    address.ip().is_loopback() && address.port() != 0
}

/// Whether `address` is exactly `127.0.0.1` or `::1` with a nonzero port: the stricter fixture
/// rule of the Responses and speech adapters, which never accept another loopback address.
pub(crate) fn is_literal_localhost(address: SocketAddr) -> bool {
    let localhost = match address.ip() {
        IpAddr::V4(ip) => ip == Ipv4Addr::LOCALHOST,
        IpAddr::V6(ip) => ip == Ipv6Addr::LOCALHOST,
    };
    localhost && address.port() != 0
}

/// Explicit positive transport bounds, each at most its `MAXIMUM_*` ceiling above.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeHttpLimits {
    pub connect_timeout: Duration,
    pub read_timeout: Duration,
    pub total_timeout: Duration,
    pub maximum_request_bytes: usize,
    pub maximum_response_bytes: usize,
    pub maximum_header_bytes: usize,
    pub maximum_chunks: usize,
}

/// The one way a caller states how a provider call is bounded: how long it may take, the largest
/// request it sends and the largest response it keeps. The connect bound, the header bound and
/// the chunk bound take the defaults every adapter used unless a call names its own, so no module
/// restates the seven numbers of [`NativeHttpLimits`]. [`NativeHttpLimits`] stays the validated
/// form the transport checks; this only builds it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProviderLimits {
    total: Duration,
    request_bytes: usize,
    response_bytes: usize,
    connect: Option<Duration>,
    read: Option<Duration>,
    header_bytes: usize,
    chunks: usize,
}

impl ProviderLimits {
    /// The connect bound defaults to five seconds, or the whole call when that is shorter.
    const DEFAULT_CONNECT: Duration = Duration::from_secs(5);
    /// Kept at 16 KiB: the media fetchers refuse a larger header bound.
    const DEFAULT_HEADER_BYTES: usize = 16 * 1024;
    /// As many chunks as the transport allows: the byte bound already limits a response, and the
    /// old 4096 refused a large body that a server sent in small frames.
    const DEFAULT_CHUNKS: usize = MAXIMUM_CHUNKS;

    /// A call that may take `total` end to end, send `request_bytes` and keep `response_bytes`.
    pub const fn new(total: Duration, request_bytes: usize, response_bytes: usize) -> Self {
        Self {
            total,
            request_bytes,
            response_bytes,
            connect: None,
            read: None,
            header_bytes: Self::DEFAULT_HEADER_BYTES,
            chunks: Self::DEFAULT_CHUNKS,
        }
    }

    /// Replaces the default connect bound.
    pub const fn connect_timeout(mut self, connect: Duration) -> Self {
        self.connect = Some(connect);
        self
    }

    /// Replaces the read bound, which otherwise equals the total.
    pub const fn read_timeout(mut self, read: Duration) -> Self {
        self.read = Some(read);
        self
    }

    /// Replaces the largest response header block kept.
    pub const fn header_bytes(mut self, bytes: usize) -> Self {
        self.header_bytes = bytes;
        self
    }

    /// Replaces the most body chunks read.
    pub const fn chunks(mut self, chunks: usize) -> Self {
        self.chunks = chunks;
        self
    }

    /// The transport form. `NativeHttp` still validates every bound when it is constructed.
    pub fn build(self) -> NativeHttpLimits {
        NativeHttpLimits {
            connect_timeout: self
                .connect
                .unwrap_or_else(|| Self::DEFAULT_CONNECT.min(self.total)),
            read_timeout: self.read.unwrap_or(self.total),
            total_timeout: self.total,
            maximum_request_bytes: self.request_bytes,
            maximum_response_bytes: self.response_bytes,
            maximum_header_bytes: self.header_bytes,
            maximum_chunks: self.chunks,
        }
    }
}

impl From<ProviderLimits> for NativeHttpLimits {
    fn from(limits: ProviderLimits) -> Self {
        limits.build()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeHttpCause {
    InvalidLimits,
    InvalidConfiguration,
    InvalidEndpoint,
    InvalidCredential,
    /// An explicitly selected provider policy refused the request before transmission.
    ProviderDisabled,
    RequestTooLarge,
    ResponseTooLarge,
    HeadersTooLarge,
    TooManyChunks,
    ContentType,
    Redirect,
    Cancelled,
    Deadline,
    Unknown,
}

impl NativeHttpCause {
    pub const ALL: [Self; 14] = [
        Self::InvalidLimits,
        Self::InvalidConfiguration,
        Self::InvalidEndpoint,
        Self::InvalidCredential,
        Self::ProviderDisabled,
        Self::RequestTooLarge,
        Self::ResponseTooLarge,
        Self::HeadersTooLarge,
        Self::TooManyChunks,
        Self::ContentType,
        Self::Redirect,
        Self::Cancelled,
        Self::Deadline,
        Self::Unknown,
    ];

    /// The closed key audit lines carry for this cause.
    pub const fn key(self) -> &'static str {
        match self {
            Self::InvalidLimits => "invalid_limits",
            Self::InvalidConfiguration => "invalid_configuration",
            Self::InvalidEndpoint => "invalid_endpoint",
            Self::InvalidCredential => "invalid_credential",
            Self::ProviderDisabled => "provider_disabled",
            Self::RequestTooLarge => "request_too_large",
            Self::ResponseTooLarge => "response_too_large",
            Self::HeadersTooLarge => "headers_too_large",
            Self::TooManyChunks => "too_many_chunks",
            Self::ContentType => "content_type",
            Self::Redirect => "redirect",
            Self::Cancelled => "cancelled",
            Self::Deadline => "deadline",
            Self::Unknown => "unknown",
        }
    }

    /// The cause whose `key()` is `key`, if any.
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|cause| cause.key() == key)
    }

    pub const fn failure_class(self) -> ProviderFailureClass {
        match self {
            Self::Cancelled => ProviderFailureClass::Cancelled,
            Self::Deadline => ProviderFailureClass::Deadline,
            Self::Unknown => ProviderFailureClass::Unknown,
            Self::InvalidLimits
            | Self::InvalidConfiguration
            | Self::InvalidEndpoint
            | Self::InvalidCredential
            | Self::ProviderDisabled
            | Self::RequestTooLarge => ProviderFailureClass::InvalidRequest,
            _ => ProviderFailureClass::Contract,
        }
    }
}

impl std::fmt::Display for NativeHttpCause {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidLimits => "the transport limits are invalid",
            Self::InvalidConfiguration => "the transport configuration is invalid",
            Self::InvalidEndpoint => "the endpoint is not allowed",
            Self::InvalidCredential => "the credential is not a valid header value",
            Self::ProviderDisabled => "the provider is disabled by policy",
            Self::RequestTooLarge => "the request is too large",
            Self::ResponseTooLarge => "the response is too large",
            Self::HeadersTooLarge => "the response headers are too large",
            Self::TooManyChunks => "the response arrived in too many chunks",
            Self::ContentType => "the response has the wrong content type",
            Self::Redirect => "the server answered with a redirect",
            Self::Cancelled => "the call was cancelled",
            Self::Deadline => "the call ran out of time",
            Self::Unknown => "the exchange failed",
        })
    }
}

impl std::error::Error for NativeHttpCause {}

/// Owned native HTTPS transport for provider calls.
pub struct NativeHttp {
    client: reqwest::Client,
    limits: NativeHttpLimits,
    openai_api_disabled: bool,
}

/// Completed provider response with bytes and supplier facts.
pub struct NativeHttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
    pub transport_request_id: Option<ProviderRequestId>,
    pub metadata: NativeHttpMetadata,
}

/// Typed transport failure with supplier facts for diagnosis.
pub struct NativeHttpError {
    pub cause: NativeHttpCause,
    pub transport_request_id: Option<ProviderRequestId>,
    pub metadata: Box<NativeHttpMetadata>,
}

/// Debug prints only the cause: supplier ids and meters stay out of logs.
impl std::fmt::Debug for NativeHttpError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NativeHttpError")
            .field("cause", &self.cause)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Display for NativeHttpError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "the provider call failed: {}", self.cause)
    }
}

/// The cause is already in the message, so it is not repeated as a source.
impl std::error::Error for NativeHttpError {}

/// Exact bounded Retry-After field values exposed by the HTTP response parser.
/// This observation alone never grants billing or resend authority.
#[derive(Clone, Default, Eq, PartialEq)]
pub struct NativeRetryAfter {
    raw_values: Vec<Vec<u8>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeRetryAfterValue {
    Absent,
    Invalid,
    Duplicate,
    DelaySeconds(u64),
    HttpDate(SystemTime),
}

impl std::fmt::Debug for NativeRetryAfter {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NativeRetryAfter")
            .field("field_count", &self.raw_values.len())
            .finish()
    }
}

impl NativeRetryAfter {
    /// Reconstructs retained evidence; callers enforce their receipt byte bounds.
    pub fn from_raw_values(raw_values: Vec<Vec<u8>>) -> Self {
        Self { raw_values }
    }

    pub fn raw_values(&self) -> &[Vec<u8>] {
        &self.raw_values
    }

    pub fn value(&self) -> NativeRetryAfterValue {
        let raw = match self.raw_values.as_slice() {
            [] => return NativeRetryAfterValue::Absent,
            [raw] => raw,
            _ => return NativeRetryAfterValue::Duplicate,
        };
        let Ok(text) = std::str::from_utf8(raw) else {
            return NativeRetryAfterValue::Invalid;
        };
        let text = text.trim_matches([' ', '\t']);
        if !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit()) {
            return text.parse().map_or(
                NativeRetryAfterValue::Invalid,
                NativeRetryAfterValue::DelaySeconds,
            );
        }
        httpdate::parse_http_date(text).map_or(
            NativeRetryAfterValue::Invalid,
            NativeRetryAfterValue::HttpDate,
        )
    }

    /// Returns the supplier minimum, or None when it cannot safely schedule a retry.
    /// A caller must additionally apply its backoff, attempt budget and absolute deadline.
    pub fn minimum_delay(&self, now: SystemTime) -> Option<Duration> {
        let delay = match self.value() {
            NativeRetryAfterValue::Absent => Duration::ZERO,
            NativeRetryAfterValue::DelaySeconds(seconds) => Duration::from_secs(seconds),
            NativeRetryAfterValue::HttpDate(date) => {
                date.duration_since(now).unwrap_or(Duration::ZERO)
            }
            NativeRetryAfterValue::Invalid | NativeRetryAfterValue::Duplicate => return None,
        };
        now.checked_add(delay)?;
        Instant::now().checked_add(delay)?;
        Some(delay)
    }
}

/// Retained supplier facts, never billing or resend authority.
#[derive(Clone, Default)]
pub struct NativeHttpMetadata {
    pub request_id: Option<ProviderRequestId>,
    pub transport_request_id: Option<ProviderRequestId>,
    pub trace_id: Option<String>,
    pub character_cost: Option<String>,
    pub retry_after: NativeRetryAfter,
}

#[derive(Clone, Copy)]
pub(crate) enum NativeResponseProfile {
    Json,
    ElevenJson,
    AudioMpeg,
}

impl NativeHttp {
    /// Validated transport bounds used to build this transport.
    pub fn limits(&self) -> NativeHttpLimits {
        self.limits
    }
    /// Builds a bounded transport; `loopback_fixture` only allows plain HTTP.
    /// It does not restrict endpoints to loopback; the wrapper must enforce that.
    /// A refusal happens before any request, so it carries no supplier facts.
    pub fn new(limits: NativeHttpLimits, loopback_fixture: bool) -> Result<Self, NativeHttpError> {
        Self::build(limits, loopback_fixture).map_err(|cause| NativeHttpError {
            cause,
            transport_request_id: None,
            metadata: Box::default(),
        })
    }

    fn build(limits: NativeHttpLimits, loopback_fixture: bool) -> Result<Self, NativeHttpCause> {
        if [
            limits.connect_timeout,
            limits.read_timeout,
            limits.total_timeout,
        ]
        .into_iter()
        .any(|value| value.is_zero() || value > MAXIMUM_EXCHANGE_TIME)
            || limits.maximum_request_bytes == 0
            || limits.maximum_request_bytes > MAXIMUM_BODY_BYTES
            || limits.maximum_response_bytes == 0
            || limits.maximum_response_bytes > MAXIMUM_BODY_BYTES
            || limits.maximum_header_bytes == 0
            || limits.maximum_header_bytes > MAXIMUM_HEADER_BYTES
            || limits.maximum_chunks == 0
            || limits.maximum_chunks > MAXIMUM_CHUNKS
        {
            return Err(NativeHttpCause::InvalidLimits);
        }
        let client = hardened_client_builder(&limits, !loopback_fixture)
            .build()
            .map_err(|_| NativeHttpCause::Unknown)?;
        Ok(Self {
            client,
            limits,
            openai_api_disabled: openai_api_block(std::env::var(DISABLE_OPENAI_API_VARIABLE))?
                | openai_api_block(std::env::var(CODEX_MODE_VARIABLE))?,
        })
    }

    /// Sends one request within `remaining_deadline` without redirects.
    pub async fn execute(
        &self,
        request: HttpRequest,
        remaining_deadline: Duration,
        cancellation: impl Future<Output = ()>,
    ) -> Result<NativeHttpResponse, NativeHttpError> {
        self.execute_profile(
            request,
            remaining_deadline,
            cancellation,
            NativeResponseProfile::Json,
        )
        .await
    }

    pub(crate) async fn execute_profile(
        &self,
        request: HttpRequest,
        remaining_deadline: Duration,
        cancellation: impl Future<Output = ()>,
        profile: NativeResponseProfile,
    ) -> Result<NativeHttpResponse, NativeHttpError> {
        let started = Instant::now();
        let (provider, operation) = (request.provider(), request.operation());
        let host = exchange_host(request.url());
        let request_bytes = request.body().len();
        let mut captured = NativeHttpMetadata::default();
        let mut trace = ExchangeTrace::default();
        let result = self
            .execute_owned(
                request,
                remaining_deadline,
                cancellation,
                profile,
                &mut captured,
                &mut trace,
            )
            .await;
        let (cause, response_bytes, metadata) = match &result {
            Ok(response) => (
                answered_cause(response.status),
                response.body.len(),
                &response.metadata,
            ),
            Err(cause) => (cause.key(), 0, &captured),
        };
        ExchangeRecord {
            provider,
            operation,
            host: &host,
            status: trace.status,
            cause,
            sent: trace.sent,
            request_bytes,
            response_bytes,
            duration: started.elapsed(),
            request_id: metadata
                .transport_request_id
                .as_ref()
                .or(metadata.request_id.as_ref())
                .map(ProviderRequestId::as_str),
        }
        .record();
        result.map_err(|cause| NativeHttpError {
            cause,
            transport_request_id: captured.transport_request_id.clone(),
            metadata: Box::new(captured),
        })
    }

    async fn execute_owned(
        &self,
        request: HttpRequest,
        remaining_deadline: Duration,
        cancellation: impl Future<Output = ()>,
        profile: NativeResponseProfile,
        captured: &mut NativeHttpMetadata,
        trace: &mut ExchangeTrace,
    ) -> Result<NativeHttpResponse, NativeHttpCause> {
        if self.openai_api_disabled && is_openai_api(request.url()) {
            return Err(NativeHttpCause::ProviderDisabled);
        }
        if request.body().len() > self.limits.maximum_request_bytes {
            return Err(NativeHttpCause::RequestTooLarge);
        }
        let duration = remaining_deadline.min(self.limits.total_timeout);
        if duration.is_zero() {
            return Err(NativeHttpCause::Deadline);
        }
        // This single future owns send and body. Dropping it on cancellation,
        // deadline or caller drop releases the response and its network work.
        let exchange = self.send_and_read(request, profile, captured, trace);
        tokio::select! {
            biased;
            () = cancellation => Err(NativeHttpCause::Cancelled),
            result = tokio::time::timeout(duration, exchange) => {
                result.map_err(|_| NativeHttpCause::Deadline)?
            }
        }
    }

    async fn send_and_read(
        &self,
        request: HttpRequest,
        profile: NativeResponseProfile,
        captured: &mut NativeHttpMetadata,
        trace: &mut ExchangeTrace,
    ) -> Result<NativeHttpResponse, NativeHttpCause> {
        let method = match request.method() {
            HttpMethod::Get => reqwest::Method::GET,
            HttpMethod::Post => reqwest::Method::POST,
            HttpMethod::Put => reqwest::Method::PUT,
        };
        let mut builder = self.client.request(method, request.url());
        for (name, value) in request.headers() {
            builder = builder.header(*name, value);
        }
        // From here the supplier may have the request, whatever happens next.
        trace.sent = SendState::PossiblySent;
        let mut response = builder
            .body(request.into_body())
            .send()
            .await
            .map_err(classify_transport_error)?;
        trace.sent = SendState::KnownSent;
        trace.status = Some(response.status().as_u16());
        if matches!(profile, NativeResponseProfile::Json) {
            // Preserve the existing Responses transport's selected-header contract.
            if let Some(value) = response.headers().get("x-request-id") {
                let value = value.to_str().map_err(|_| NativeHttpCause::Unknown)?;
                if value.len() > MAXIMUM_SUPPLIER_ID_BYTES {
                    return Err(NativeHttpCause::HeadersTooLarge);
                }
                captured.transport_request_id =
                    Some(ProviderRequestId::new(value).map_err(|_| NativeHttpCause::Unknown)?);
            }
        } else {
            for name in ["x-request-id", "request-id", "x-trace-id", "character-cost"] {
                let mut values = response.headers().get_all(name).iter();
                if let Some(value) = values.next() {
                    if values.next().is_some() {
                        return Err(NativeHttpCause::HeadersTooLarge);
                    }
                    let value = value.to_str().map_err(|_| NativeHttpCause::Unknown)?;
                    if value.is_empty()
                        || value.len() > MAXIMUM_SUPPLIER_ID_BYTES
                        || !value.bytes().all(is_visible_ascii)
                    {
                        return Err(NativeHttpCause::HeadersTooLarge);
                    }
                    match name {
                        "x-request-id" => {
                            captured.transport_request_id = Some(
                                ProviderRequestId::new(value)
                                    .map_err(|_| NativeHttpCause::Unknown)?,
                            )
                        }
                        "request-id" => {
                            captured.request_id = Some(
                                ProviderRequestId::new(value)
                                    .map_err(|_| NativeHttpCause::Unknown)?,
                            )
                        }
                        "x-trace-id" => captured.trace_id = Some(value.to_owned()),
                        _ => captured.character_cost = Some(value.to_owned()),
                    }
                }
            }
        }
        let status = response.status().as_u16();
        if response.status().is_redirection() {
            return Err(NativeHttpCause::Redirect);
        }
        if header_block_bytes(response.headers())
            .is_none_or(|bytes| bytes > self.limits.maximum_header_bytes)
        {
            return Err(NativeHttpCause::HeadersTooLarge);
        }
        captured.retry_after = NativeRetryAfter::from_raw_values(
            response
                .headers()
                .get_all(reqwest::header::RETRY_AFTER)
                .iter()
                .map(|value| value.as_bytes().to_vec())
                .collect(),
        );
        let mut content_types = response
            .headers()
            .get_all(reqwest::header::CONTENT_TYPE)
            .iter();
        let content_type = content_types
            .next()
            .and_then(|value| value.to_str().ok())
            .ok_or(NativeHttpCause::ContentType)?;
        let expected_type = match (profile, status) {
            (NativeResponseProfile::AudioMpeg, 200) => "audio/mpeg",
            _ => "application/json",
        };
        if content_types.next().is_some()
            || !content_type
                .split(';')
                .next()
                .is_some_and(|media| media.trim().eq_ignore_ascii_case(expected_type))
        {
            return Err(NativeHttpCause::ContentType);
        }
        if response
            .content_length()
            .is_some_and(|length| length > self.limits.maximum_response_bytes as u64)
        {
            return Err(NativeHttpCause::ResponseTooLarge);
        }
        let body = read_bounded_body(
            &mut response,
            self.limits.maximum_response_bytes,
            self.limits.maximum_chunks,
        )
        .await?;
        Ok(NativeHttpResponse {
            status,
            body,
            transport_request_id: captured.transport_request_id.clone(),
            metadata: captured.clone(),
        })
    }
}

/// The one hardened `reqwest` posture every native client in this crate starts from: rustls,
/// no retries, no redirects, no referer, no proxy, and the connect, read and total bounds of
/// `limits`. Callers add only what differs (a pinned resolver, a connection pool bound).
pub(crate) fn hardened_client_builder(
    limits: &NativeHttpLimits,
    https_only: bool,
) -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .tls_backend_rustls()
        .https_only(https_only)
        .retry(reqwest::retry::never())
        .redirect(reqwest::redirect::Policy::none())
        .referer(false)
        .connection_verbose(false)
        .no_proxy()
        .connect_timeout(limits.connect_timeout)
        .read_timeout(limits.read_timeout)
        .timeout(limits.total_timeout)
}

/// How far a provider request got toward the supplier, as audit lines say (`send_state=`).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SendState {
    /// Refused or stopped before the request left this process.
    #[default]
    Unsent,
    /// The send began but no answer arrived: the supplier may have the request.
    PossiblySent,
    /// The supplier answered with a status.
    KnownSent,
}

impl SendState {
    pub const ALL: [Self; 3] = [Self::Unsent, Self::PossiblySent, Self::KnownSent];

    pub const fn key(self) -> &'static str {
        match self {
            Self::Unsent => "unsent",
            Self::PossiblySent => "possibly_sent",
            Self::KnownSent => "known_sent",
        }
    }

    /// The state whose `key()` is `key`, if any.
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|state| state.key() == key)
    }
}

/// What one exchange reached, kept outside the exchange future so a cancelled or timed-out
/// exchange still says how far it got.
#[derive(Default)]
pub(crate) struct ExchangeTrace {
    pub(crate) sent: SendState,
    pub(crate) status: Option<u16>,
}

/// The cause key of an exchange the supplier answered: `ok` below 400, else the status class.
pub(crate) const fn answered_cause(status: u16) -> &'static str {
    match status {
        429 => "rate_limited",
        400..=499 => "rejected",
        500.. => "supplier_error",
        _ => "ok",
    }
}

/// The host of `url` for an audit line: never its path, query or credentials; `-` when it has
/// none.
pub(crate) fn exchange_host(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .unwrap_or_else(|| "-".to_owned())
}

/// One provider exchange, as its `PROVIDER exchange` audit line describes it: closed keys,
/// numbers, the endpoint host and the supplier's request id. Never a URL path or query, a
/// credential or a body.
pub(crate) struct ExchangeRecord<'a> {
    pub(crate) provider: ProviderName,
    pub(crate) operation: Option<&'static str>,
    pub(crate) host: &'a str,
    pub(crate) status: Option<u16>,
    pub(crate) cause: &'static str,
    pub(crate) sent: SendState,
    pub(crate) request_bytes: usize,
    pub(crate) response_bytes: usize,
    pub(crate) duration: Duration,
    pub(crate) request_id: Option<&'a str>,
}

impl ExchangeRecord<'_> {
    /// Writes the line through the shared audit path.
    pub(crate) fn record(&self) {
        df_observe::record_audit_line("PROVIDER", self.line());
    }

    fn line(&self) -> String {
        let operation = self
            .operation
            .filter(|label| {
                !label.is_empty()
                    && label
                        .bytes()
                        .all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'_'))
            })
            .unwrap_or("-");
        let status = self
            .status
            .map_or_else(|| "-".to_owned(), |status| status.to_string());
        let request_id = self
            .request_id
            .filter(|id| {
                !id.is_empty()
                    && id.len() <= MAXIMUM_SUPPLIER_ID_BYTES
                    && id.bytes().all(is_visible_ascii)
            })
            .unwrap_or("-");
        format!(
            "PROVIDER exchange provider={} operation={operation} host={} status={status} \
             cause={} send_state={} request_bytes={} response_bytes={} duration_ms={} \
             request_id={request_id}",
            self.provider.key(),
            self.host,
            self.cause,
            self.sent.key(),
            self.request_bytes,
            self.response_bytes,
            saturating_millis(self.duration),
        )
    }
}

/// The size of a response header block (names plus values), in bytes; `None` on overflow.
pub(crate) fn header_block_bytes(headers: &reqwest::header::HeaderMap) -> Option<usize> {
    headers.iter().try_fold(0usize, |total, (name, value)| {
        total
            .checked_add(name.as_str().len())?
            .checked_add(value.as_bytes().len())
    })
}

/// Reads a response body through its end, refusing more than `maximum_bytes` or more than
/// `maximum_chunks` chunks. Only a clean end of body (`chunk() == None`) returns bytes.
pub(crate) async fn read_bounded_body(
    response: &mut reqwest::Response,
    maximum_bytes: usize,
    maximum_chunks: usize,
) -> Result<Vec<u8>, NativeHttpCause> {
    let mut body = Vec::new();
    let mut chunks = 0usize;
    while let Some(chunk) = response.chunk().await.map_err(classify_transport_error)? {
        chunks += 1;
        if chunks > maximum_chunks {
            return Err(NativeHttpCause::TooManyChunks);
        }
        if body
            .len()
            .checked_add(chunk.len())
            .is_none_or(|length| length > maximum_bytes)
        {
            return Err(NativeHttpCause::ResponseTooLarge);
        }
        body.try_reserve(chunk.len())
            .map_err(|_| NativeHttpCause::ResponseTooLarge)?;
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// A request body writer that refuses to grow past `maximum_bytes`, so a serializer stops at the
/// transport's request bound instead of building a body the transport would refuse.
pub(crate) struct BoundedRequestBody {
    bytes: Vec<u8>,
    maximum_bytes: usize,
}

impl BoundedRequestBody {
    pub(crate) fn new(maximum_bytes: usize) -> Self {
        Self {
            bytes: Vec::new(),
            maximum_bytes,
        }
    }

    pub(crate) fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

impl std::io::Write for BoundedRequestBody {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let length = self
            .bytes
            .len()
            .checked_add(bytes.len())
            .ok_or_else(|| std::io::Error::other("request body bound"))?;
        if length > self.maximum_bytes {
            return Err(std::io::Error::other("request body bound"));
        }
        self.bytes
            .try_reserve(bytes.len())
            .map_err(|_| std::io::Error::other("request body capacity"))?;
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn openai_api_block(value: Result<String, std::env::VarError>) -> Result<bool, NativeHttpCause> {
    match value {
        Ok(value) if value == "on" => Ok(true),
        Ok(value) if value == "off" => Ok(false),
        Err(std::env::VarError::NotPresent) => Ok(false),
        _ => Err(NativeHttpCause::InvalidConfiguration),
    }
}

fn is_openai_api(url: &str) -> bool {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .is_some_and(|host| {
            host.trim_end_matches('.')
                .eq_ignore_ascii_case("api.openai.com")
        })
}

/// A transport timeout is a deadline; every other `reqwest` failure is of unknown outcome.
pub(crate) fn classify_transport_error(error: reqwest::Error) -> NativeHttpCause {
    if error.is_timeout() {
        NativeHttpCause::Deadline
    } else {
        NativeHttpCause::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_openai_api_authority_without_blocking_other_suppliers() {
        assert!(is_openai_api("https://API.OPENAI.COM./v1/responses"));
        assert!(!is_openai_api(
            "https://api.openai.com.example.test/v1/responses"
        ));
        assert!(!is_openai_api(
            "https://api.elevenlabs.io/v1/text-to-speech"
        ));
        assert!(!is_openai_api("http://127.0.0.1:8000/v1/responses"));
    }

    #[test]
    fn openai_api_block_refuses_unrecognized_configuration() {
        assert_eq!(openai_api_block(Ok("on".into())), Ok(true));
        assert_eq!(openai_api_block(Ok("off".into())), Ok(false));
        assert_eq!(
            openai_api_block(Err(std::env::VarError::NotPresent)),
            Ok(false)
        );
        assert_eq!(
            openai_api_block(Ok("enabled".into())),
            Err(NativeHttpCause::InvalidConfiguration)
        );
    }

    #[test]
    fn configured_process_mode_is_applied_to_new_transports() {
        if let Ok(value) = std::env::var("DF_TEST_HTTP_BLOCK_MODE") {
            let limits = ProviderLimits::new(Duration::from_secs(1), 1024, 1024).build();
            let result = NativeHttp::new(limits, false);
            match value.as_str() {
                "on" => assert!(result.unwrap().openai_api_disabled),
                "off" => assert!(!result.unwrap().openai_api_disabled),
                "invalid" => assert!(matches!(
                    result,
                    Err(NativeHttpError {
                        cause: NativeHttpCause::InvalidConfiguration,
                        ..
                    })
                )),
                _ => panic!("unexpected test mode"),
            }
            return;
        }
        for mode in ["on", "off", "invalid"] {
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "native_http::tests::configured_process_mode_is_applied_to_new_transports",
                ])
                .env("DF_TEST_HTTP_BLOCK_MODE", mode)
                .env(DISABLE_OPENAI_API_VARIABLE, mode)
                .status()
                .unwrap();
            assert!(status.success(), "child failed for {mode}");
        }
    }

    #[test]
    fn codex_mode_alone_still_blocks_openai() {
        if std::env::var_os("DF_TEST_CODEX_GUARD_CHILD").is_some() {
            let limits = ProviderLimits::new(Duration::from_secs(1), 1024, 1024).build();
            assert!(NativeHttp::new(limits, false).unwrap().openai_api_disabled);
            return;
        }
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "native_http::tests::codex_mode_alone_still_blocks_openai",
            ])
            .env("DF_TEST_CODEX_GUARD_CHILD", "1")
            .env(CODEX_MODE_VARIABLE, "on")
            .env_remove(DISABLE_OPENAI_API_VARIABLE)
            .status()
            .unwrap();
        assert!(status.success());
    }

    #[tokio::test]
    async fn openai_api_block_refuses_all_openai_adapter_endpoints_before_sending() {
        let limits = ProviderLimits::new(Duration::from_secs(1), 1024, 1024).build();
        let mut transport = NativeHttp::new(limits, false).unwrap();
        transport.openai_api_disabled = true;
        for path in [
            "responses",
            "images/generations",
            "images/edits",
            "embeddings",
            "chat/completions",
        ] {
            let request = HttpRequest::bearer_native(
                &format!("https://api.openai.com/v1/{path}"),
                "test-credential",
                "application/json".to_owned(),
                Vec::new(),
            );
            let failure = transport
                .execute(request, Duration::from_secs(1), std::future::pending())
                .await
                .err()
                .expect("policy must reject before attempting a connection");
            assert_eq!(failure.cause, NativeHttpCause::ProviderDisabled);
        }
    }
}
