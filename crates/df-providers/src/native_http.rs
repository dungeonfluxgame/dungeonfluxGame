//! Owned native HTTPS execution. No redirects, proxies, resends, or detached tasks.

use std::{future::Future, time::Duration};

use crate::{HttpMethod, HttpRequest, ProviderFailureClass, ProviderRequestId};

/// Explicit positive transport bounds. Connect/read/total bounds cannot exceed
/// 180 seconds; response retention is at most 16 MiB and headers at most 64 KiB.
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
    const DEFAULT_HEADER_BYTES: usize = 16 * 1024;
    const DEFAULT_CHUNKS: usize = 4096;

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
pub enum NativeHttpRefusal {
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

impl NativeHttpRefusal {
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

pub(crate) struct NativeHttp {
    client: reqwest::Client,
    limits: NativeHttpLimits,
    openai_api_disabled: bool,
}

pub(crate) struct NativeHttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
    pub transport_request_id: Option<ProviderRequestId>,
    pub metadata: NativeHttpMetadata,
}

pub(crate) struct NativeHttpFailure {
    pub reason: NativeHttpRefusal,
    pub transport_request_id: Option<ProviderRequestId>,
    pub metadata: Box<NativeHttpMetadata>,
}

/// Retained supplier facts, never billing or resend authority.
#[derive(Clone, Default)]
pub(crate) struct NativeHttpMetadata {
    pub request_id: Option<ProviderRequestId>,
    pub transport_request_id: Option<ProviderRequestId>,
    pub trace_id: Option<String>,
    pub character_cost: Option<String>,
}

#[derive(Clone, Copy)]
pub(crate) enum NativeResponseProfile {
    Json,
    ElevenJson,
    AudioMpeg,
}

impl NativeHttp {
    pub(crate) fn limits(&self) -> NativeHttpLimits {
        self.limits
    }
    pub(crate) fn new(
        limits: NativeHttpLimits,
        loopback_fixture: bool,
    ) -> Result<Self, NativeHttpRefusal> {
        let ceiling = Duration::from_secs(180);
        if [
            limits.connect_timeout,
            limits.read_timeout,
            limits.total_timeout,
        ]
        .into_iter()
        .any(|value| value.is_zero() || value > ceiling)
            || limits.maximum_request_bytes == 0
            || limits.maximum_request_bytes > 16_777_216
            || limits.maximum_response_bytes == 0
            || limits.maximum_response_bytes > 16_777_216
            || limits.maximum_header_bytes == 0
            || limits.maximum_header_bytes > 65_536
            || limits.maximum_chunks == 0
            || limits.maximum_chunks > 65_536
        {
            return Err(NativeHttpRefusal::InvalidLimits);
        }
        let client = reqwest::Client::builder()
            .tls_backend_rustls()
            .https_only(!loopback_fixture)
            .retry(reqwest::retry::never())
            .redirect(reqwest::redirect::Policy::none())
            .referer(false)
            .connection_verbose(false)
            .no_proxy()
            .connect_timeout(limits.connect_timeout)
            .read_timeout(limits.read_timeout)
            .timeout(limits.total_timeout)
            .build()
            .map_err(|_| NativeHttpRefusal::Unknown)?;
        Ok(Self {
            client,
            limits,
            openai_api_disabled: openai_api_block(std::env::var("DF_GAMEPLAY_DISABLE_OPENAI_API"))?
                | openai_api_block(std::env::var("DF_GAMEPLAY_CODEX_MODE"))?,
        })
    }

    pub(crate) async fn execute(
        &self,
        request: HttpRequest,
        remaining_deadline: Duration,
        cancellation: impl Future<Output = ()>,
    ) -> Result<NativeHttpResponse, NativeHttpFailure> {
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
    ) -> Result<NativeHttpResponse, NativeHttpFailure> {
        let mut captured = NativeHttpMetadata::default();
        let result = self
            .execute_owned(
                request,
                remaining_deadline,
                cancellation,
                profile,
                &mut captured,
            )
            .await;
        result.map_err(|reason| NativeHttpFailure {
            reason,
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
    ) -> Result<NativeHttpResponse, NativeHttpRefusal> {
        if self.openai_api_disabled && is_openai_api(request.url()) {
            return Err(NativeHttpRefusal::ProviderDisabled);
        }
        if request.body().len() > self.limits.maximum_request_bytes {
            return Err(NativeHttpRefusal::RequestTooLarge);
        }
        let duration = remaining_deadline.min(self.limits.total_timeout);
        if duration.is_zero() {
            return Err(NativeHttpRefusal::Deadline);
        }
        // This single future owns send and body. Dropping it on cancellation,
        // deadline or caller drop releases the response and its network work.
        tokio::select! {
            biased;
            () = cancellation => Err(NativeHttpRefusal::Cancelled),
            result = tokio::time::timeout(duration, self.send_and_read(request, profile, captured)) => {
                result.map_err(|_| NativeHttpRefusal::Deadline)?
            }
        }
    }

    async fn send_and_read(
        &self,
        request: HttpRequest,
        profile: NativeResponseProfile,
        captured: &mut NativeHttpMetadata,
    ) -> Result<NativeHttpResponse, NativeHttpRefusal> {
        let method = match request.method() {
            HttpMethod::Get => reqwest::Method::GET,
            HttpMethod::Post => reqwest::Method::POST,
            HttpMethod::Put => reqwest::Method::PUT,
        };
        let mut builder = self.client.request(method, request.url());
        for (name, value) in request.headers() {
            builder = builder.header(*name, value);
        }
        let mut response = builder
            .body(request.into_body())
            .send()
            .await
            .map_err(classify_transport_error)?;
        if matches!(profile, NativeResponseProfile::Json) {
            // Preserve the existing Responses transport's selected-header contract.
            if let Some(value) = response.headers().get("x-request-id") {
                let value = value.to_str().map_err(|_| NativeHttpRefusal::Unknown)?;
                if value.len() > 256 {
                    return Err(NativeHttpRefusal::HeadersTooLarge);
                }
                captured.transport_request_id =
                    Some(ProviderRequestId::new(value).map_err(|_| NativeHttpRefusal::Unknown)?);
            }
        } else {
            for name in ["x-request-id", "request-id", "x-trace-id", "character-cost"] {
                let mut values = response.headers().get_all(name).iter();
                if let Some(value) = values.next() {
                    if values.next().is_some() {
                        return Err(NativeHttpRefusal::HeadersTooLarge);
                    }
                    let value = value.to_str().map_err(|_| NativeHttpRefusal::Unknown)?;
                    if value.is_empty()
                        || value.len() > 256
                        || !value.bytes().all(|b| (0x21..=0x7e).contains(&b))
                    {
                        return Err(NativeHttpRefusal::HeadersTooLarge);
                    }
                    match name {
                        "x-request-id" => {
                            captured.transport_request_id = Some(
                                ProviderRequestId::new(value)
                                    .map_err(|_| NativeHttpRefusal::Unknown)?,
                            )
                        }
                        "request-id" => {
                            captured.request_id = Some(
                                ProviderRequestId::new(value)
                                    .map_err(|_| NativeHttpRefusal::Unknown)?,
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
            return Err(NativeHttpRefusal::Redirect);
        }
        let mut header_bytes = 0usize;
        for (name, value) in response.headers() {
            header_bytes = header_bytes
                .checked_add(name.as_str().len())
                .and_then(|count| count.checked_add(value.as_bytes().len()))
                .ok_or(NativeHttpRefusal::HeadersTooLarge)?;
        }
        if header_bytes > self.limits.maximum_header_bytes {
            return Err(NativeHttpRefusal::HeadersTooLarge);
        }
        let mut content_types = response
            .headers()
            .get_all(reqwest::header::CONTENT_TYPE)
            .iter();
        let content_type = content_types
            .next()
            .and_then(|value| value.to_str().ok())
            .ok_or(NativeHttpRefusal::ContentType)?;
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
            return Err(NativeHttpRefusal::ContentType);
        }
        if response
            .content_length()
            .is_some_and(|length| length > self.limits.maximum_response_bytes as u64)
        {
            return Err(NativeHttpRefusal::ResponseTooLarge);
        }
        let mut body = Vec::new();
        let mut chunks = 0usize;
        while let Some(chunk) = response.chunk().await.map_err(classify_transport_error)? {
            chunks += 1;
            if chunks > self.limits.maximum_chunks {
                return Err(NativeHttpRefusal::TooManyChunks);
            }
            let length = body
                .len()
                .checked_add(chunk.len())
                .ok_or(NativeHttpRefusal::ResponseTooLarge)?;
            if length > self.limits.maximum_response_bytes {
                return Err(NativeHttpRefusal::ResponseTooLarge);
            }
            body.try_reserve(chunk.len())
                .map_err(|_| NativeHttpRefusal::ResponseTooLarge)?;
            body.extend_from_slice(&chunk);
        }
        // Only actual chunk() == None produces this complete observation.
        Ok(NativeHttpResponse {
            status,
            body,
            transport_request_id: captured.transport_request_id.clone(),
            metadata: captured.clone(),
        })
    }
}

fn openai_api_block(value: Result<String, std::env::VarError>) -> Result<bool, NativeHttpRefusal> {
    match value {
        Ok(value) if value == "on" => Ok(true),
        Ok(value) if value == "off" => Ok(false),
        Err(std::env::VarError::NotPresent) => Ok(false),
        _ => Err(NativeHttpRefusal::InvalidConfiguration),
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

fn classify_transport_error(error: reqwest::Error) -> NativeHttpRefusal {
    if error.is_timeout() {
        NativeHttpRefusal::Deadline
    } else {
        NativeHttpRefusal::Unknown
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
            Err(NativeHttpRefusal::InvalidConfiguration)
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
                    Err(NativeHttpRefusal::InvalidConfiguration)
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
                .env("DF_GAMEPLAY_DISABLE_OPENAI_API", mode)
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
            .env("DF_GAMEPLAY_CODEX_MODE", "on")
            .env_remove("DF_GAMEPLAY_DISABLE_OPENAI_API")
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
            assert_eq!(failure.reason, NativeHttpRefusal::ProviderDisabled);
        }
    }
}
