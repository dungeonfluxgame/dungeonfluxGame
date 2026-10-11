//! Private, deterministic HTTP field mapping for the two dated provider routes.
//!
//! These builders describe requests; they do not dispatch them or qualify a
//! candidate for dispatch. Credentials and body data are redacted from Debug.
//! Schema sources observed 2026-10-10:
//! - <https://elevenlabs.io/docs/api-reference/text-to-speech/stream>
//! - <https://elevenlabs.io/docs/api-reference/introduction/>
//! - <https://fal.ai/models/fal-ai/flux/schnell/api>
//! - <https://fal.ai/docs/documentation/model-apis/inference/queue>

use std::fmt;

const ELEVEN_BASE: &str = "https://api.elevenlabs.io";
const ELEVEN_MODEL: &str = "eleven_flash_v2_5";
const ELEVEN_OUTPUT_FORMAT: &str = "mp3_22050_32";
const FAL_QUEUE_BASE: &str = "https://queue.fal.run/fal-ai/flux/schnell";

/// HTTP method used by the generic mapped request value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HttpMethod {
    Get,
    Post,
}

impl HttpMethod {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
        }
    }
}

/// A generic request assembled from a selected supplier HTTP schema.
///
/// Header values and body bytes are available to the transport owner, but the
/// custom formatter never prints them.
#[derive(Clone, Eq, PartialEq)]
pub struct HttpRequest {
    method: HttpMethod,
    url: String,
    headers: Vec<(&'static str, String)>,
    body: Vec<u8>,
}

impl HttpRequest {
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn elevenlabs_native(
        endpoint: &str,
        credential: &str,
        content_type: String,
        body: Vec<u8>,
    ) -> Self {
        Self {
            method: HttpMethod::Post,
            url: endpoint.to_owned(),
            headers: vec![
                ("xi-api-key", credential.to_owned()),
                ("Content-Type", content_type),
            ],
            body,
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn headers(&self) -> &[(&'static str, String)] {
        &self.headers
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn responses(endpoint: &str, credential: &str, body: Vec<u8>) -> Self {
        Self {
            method: HttpMethod::Post,
            url: endpoint.to_owned(),
            headers: vec![
                ("Authorization", format!("Bearer {credential}")),
                ("Content-Type", "application/json".to_owned()),
            ],
            body,
        }
    }

    pub const fn method(&self) -> HttpMethod {
        self.method
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(header_name, _)| header_name.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    pub fn body(&self) -> &[u8] {
        &self.body
    }
}

impl fmt::Debug for HttpRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HttpRequest")
            .field("method", &self.method)
            .field("url", &"[REDACTED]")
            .field("headers", &"[REDACTED]")
            .field("body", &"[REDACTED]")
            .finish()
    }
}

/// Safe, non-payload error from constructing or validating a provider mapping.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HttpRequestError {
    EmptyCredential,
    EmptyVoiceId,
    EmptyText,
    EmptyPrompt,
    EmptyProviderRequestId,
    InvalidSupplierUrl,
    ResultBeforeCompletion,
    EmptyImages,
    InvalidImageUrl,
    InvalidImageContentType,
    InvalidImageDimensions,
    UnsuccessfulAudioStatus,
    InvalidAudioContentType,
    IncompleteAudioStream,
    EmptyAudioBody,
}

/// Supplier-generated request identity. This is intentionally distinct from
/// the application-owned OperationId and is never used as one.
#[derive(Clone, Eq, PartialEq)]
pub struct ProviderRequestId(String);

impl ProviderRequestId {
    pub fn new(value: &str) -> Result<Self, HttpRequestError> {
        if value.trim().is_empty() || value.trim() != value {
            return Err(HttpRequestError::EmptyProviderRequestId);
        }
        Ok(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ProviderRequestId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ProviderRequestId([REDACTED])")
    }
}

/// Normalized queue outcome associated with a positive provider request ID.
#[derive(Clone, PartialEq)]
pub enum FalQueueObservation {
    Pending(ProviderRequestId),
    Completed {
        request_id: ProviderRequestId,
        /// Supplier-reported status metric in seconds, not verified billing.
        inference_time: Option<f64>,
    },
    Failed {
        request_id: ProviderRequestId,
        error_type: Option<String>,
        error: Option<String>,
    },
    Unknown(ProviderRequestId),
}

impl FalQueueObservation {
    /// Supplier-reported FAL status metric in seconds, not billed usage.
    pub fn inference_time_seconds(&self) -> Option<f64> {
        match self {
            Self::Completed { inference_time, .. } => *inference_time,
            _ => None,
        }
    }
}

impl fmt::Debug for FalQueueObservation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Pending(_) => "Pending",
            Self::Completed { .. } => "Completed",
            Self::Failed { .. } => "Failed",
            Self::Unknown(_) => "Unknown",
        };
        formatter.debug_tuple(name).field(&"[REDACTED]").finish()
    }
}

/// Whether a FAL submit response positively identified the accepted request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FalSubmitObservation {
    Accepted(ProviderRequestId),
    Unknown,
}

/// A normalized image item from the documented FAL result schema.
#[derive(Clone, Eq, PartialEq)]
pub struct ProviderImageMetadata {
    pub url: String,
    pub content_type: String,
    pub width: u32,
    pub height: u32,
}

impl fmt::Debug for ProviderImageMetadata {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderImageMetadata")
            .field("url", &"[REDACTED]")
            .field("content_type", &self.content_type)
            .field("width", &self.width)
            .field("height", &self.height)
            .finish()
    }
}

/// Validated facts from a complete ElevenLabs streaming response.
#[derive(Clone, Eq, PartialEq)]
pub struct ElevenLabsAudioObservation {
    request_id: Option<String>,
    trace_id: Option<String>,
    character_cost: Option<String>,
    byte_count: usize,
}

impl ElevenLabsAudioObservation {
    pub fn request_id(&self) -> Option<&str> {
        self.request_id.as_deref()
    }

    pub fn trace_id(&self) -> Option<&str> {
        self.trace_id.as_deref()
    }

    /// Supplier-reported character metadata, not verified billed usage.
    pub fn character_cost(&self) -> Option<&str> {
        self.character_cost.as_deref()
    }

    pub const fn byte_count(&self) -> usize {
        self.byte_count
    }
}

impl fmt::Debug for ElevenLabsAudioObservation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ElevenLabsAudioObservation")
            .field(
                "request_id",
                &self.request_id.as_ref().map(|_| "[REDACTED]"),
            )
            .field("trace_id", &self.trace_id.as_ref().map(|_| "[REDACTED]"))
            .field(
                "character_cost",
                &self.character_cost.as_ref().map(|_| "[REDACTED]"),
            )
            .field("byte_count", &self.byte_count)
            .finish()
    }
}

/// Builds the selected ElevenLabs streaming TTS request shape.
///
/// `voice_id` is encoded as one path segment. The selected output format is
/// explicit because availability is plan-dependent. This function performs no
/// network operation and does not establish entitlement.
pub fn elevenlabs_stream_request(
    voice_id: &str,
    api_key: &str,
    text: &str,
) -> Result<HttpRequest, HttpRequestError> {
    if voice_id.trim().is_empty() {
        return Err(HttpRequestError::EmptyVoiceId);
    }
    if api_key.is_empty() {
        return Err(HttpRequestError::EmptyCredential);
    }
    if text.trim().is_empty() {
        return Err(HttpRequestError::EmptyText);
    }

    let url = format!(
        "{ELEVEN_BASE}/v1/text-to-speech/{}/stream?output_format={ELEVEN_OUTPUT_FORMAT}",
        encode_path_segment(voice_id)
    );
    let body = format!(
        "{{\"text\":{},\"model_id\":{}}}",
        json_string(text),
        json_string(ELEVEN_MODEL)
    );

    Ok(HttpRequest {
        method: HttpMethod::Post,
        url,
        headers: vec![
            ("xi-api-key", api_key.to_owned()),
            ("Content-Type", "application/json".to_owned()),
        ],
        body: body.into_bytes(),
    })
}

/// Builds the selected FAL FLUX Schnell queue submission request shape.
///
/// The small input shape uses documented fields and fixed, bounded example
/// values. It performs no network operation and does not establish entitlement.
pub fn fal_submit_request(api_key: &str, prompt: &str) -> Result<HttpRequest, HttpRequestError> {
    if api_key.is_empty() {
        return Err(HttpRequestError::EmptyCredential);
    }
    if prompt.trim().is_empty() {
        return Err(HttpRequestError::EmptyPrompt);
    }

    let body = format!(
        "{{\"prompt\":{},\"num_inference_steps\":4,\"image_size\":\"landscape_4_3\",\"num_images\":1,\"enable_safety_checker\":true,\"output_format\":\"jpeg\"}}",
        json_string(prompt)
    );
    Ok(HttpRequest {
        method: HttpMethod::Post,
        url: FAL_QUEUE_BASE.to_owned(),
        headers: vec![
            ("Authorization", format!("Key {api_key}")),
            ("Content-Type", "application/json".to_owned()),
        ],
        body: body.into_bytes(),
    })
}

/// Validates the selected MP3 stream and preserves optional supplier metadata.
///
/// `body_complete` must mean the HTTP stream reached a clean EOF. No partial
/// stream is reported as successful, and header metadata is not billing proof.
pub fn validate_elevenlabs_audio(
    status_code: u16,
    content_type: &str,
    body_complete: bool,
    body_byte_count: usize,
    request_id: Option<&str>,
    trace_id: Option<&str>,
    character_cost: Option<&str>,
) -> Result<ElevenLabsAudioObservation, HttpRequestError> {
    if status_code != 200 {
        return Err(HttpRequestError::UnsuccessfulAudioStatus);
    }
    let media_type = content_type
        .split_once(';')
        .map_or(content_type, |(media_type, _)| media_type)
        .trim();
    if !media_type.eq_ignore_ascii_case("audio/mpeg") {
        return Err(HttpRequestError::InvalidAudioContentType);
    }
    if !body_complete {
        return Err(HttpRequestError::IncompleteAudioStream);
    }
    if body_byte_count == 0 {
        return Err(HttpRequestError::EmptyAudioBody);
    }
    Ok(ElevenLabsAudioObservation {
        request_id: nonempty_header(request_id),
        trace_id: nonempty_header(trace_id),
        character_cost: nonempty_header(character_cost),
        byte_count: body_byte_count,
    })
}

/// Classifies a FAL status response only when it identifies the expected request.
///
/// An absent submit response is not accepted here; callers must retain that
/// dispatch as outcome-unknown. The function never authorizes a resend.
pub fn classify_fal_status(
    expected_request_id: &ProviderRequestId,
    observed_request_id: &str,
    status: &str,
    error_type: Option<&str>,
    error: Option<&str>,
    inference_time_seconds: Option<f64>,
) -> Result<FalQueueObservation, HttpRequestError> {
    if expected_request_id.as_str() != observed_request_id {
        return Ok(FalQueueObservation::Unknown(expected_request_id.clone()));
    }

    let request_id = expected_request_id.clone();
    match status {
        "IN_QUEUE" | "IN_PROGRESS" if error_type.is_none() && error.is_none() => {
            Ok(FalQueueObservation::Pending(request_id))
        }
        "IN_QUEUE" | "IN_PROGRESS" => Ok(FalQueueObservation::Unknown(request_id)),
        "COMPLETED" if error_type.is_some() || error.is_some() => Ok(FalQueueObservation::Failed {
            request_id,
            error_type: error_type.map(str::to_owned),
            error: error.map(str::to_owned),
        }),
        "COMPLETED" => {
            if inference_time_seconds.is_some_and(|value| !value.is_finite() || value < 0.0) {
                return Ok(FalQueueObservation::Unknown(request_id));
            }
            Ok(FalQueueObservation::Completed {
                request_id,
                inference_time: inference_time_seconds,
            })
        }
        _ => Ok(FalQueueObservation::Unknown(request_id)),
    }
}

/// Keeps non-success status lookups ambiguous instead of treating absence as failure.
pub fn classify_fal_status_response(
    http_status: u16,
    expected_request_id: &ProviderRequestId,
    observed_request_id: Option<&str>,
    status: Option<&str>,
    error_type: Option<&str>,
    error: Option<&str>,
    inference_time_seconds: Option<f64>,
) -> Result<FalQueueObservation, HttpRequestError> {
    if http_status != 200 {
        return Ok(FalQueueObservation::Unknown(expected_request_id.clone()));
    }
    let Some(observed_request_id) = observed_request_id else {
        return Ok(FalQueueObservation::Unknown(expected_request_id.clone()));
    };
    let Some(status) = status else {
        return Ok(FalQueueObservation::Unknown(expected_request_id.clone()));
    };
    classify_fal_status(
        expected_request_id,
        observed_request_id,
        status,
        error_type,
        error,
        inference_time_seconds,
    )
}

/// Classifies a FAL submit response using its documented identity and URLs.
///
/// Missing, malformed, or non-success responses remain `Unknown`: the caller
/// must preserve possible dispatch and must not automatically resend.
pub fn classify_fal_submit_response(
    status_code: u16,
    request_id: Option<&str>,
    status_url: Option<&str>,
    response_url: Option<&str>,
) -> FalSubmitObservation {
    let (Some(request_id), Some(status_url), Some(response_url)) =
        (request_id, status_url, response_url)
    else {
        return FalSubmitObservation::Unknown;
    };
    if status_code != 200 || request_id.trim().is_empty() {
        return FalSubmitObservation::Unknown;
    }
    let Ok(request_id) = ProviderRequestId::new(request_id) else {
        return FalSubmitObservation::Unknown;
    };
    if status_url != expected_fal_status_url(&request_id)
        || response_url != expected_fal_response_url(&request_id)
    {
        return FalSubmitObservation::Unknown;
    }
    FalSubmitObservation::Accepted(request_id)
}

/// Validates normalized FAL image metadata only for the same completed request.
///
/// The HTTP/JSON transport owner must deserialize the documented response fields
/// before calling this function. Returned URLs are restricted to fal media hosts.
pub fn validate_fal_images(
    observation: &FalQueueObservation,
    expected_request_id: &ProviderRequestId,
    images: &[ProviderImageMetadata],
) -> Result<(), HttpRequestError> {
    match observation {
        FalQueueObservation::Completed { request_id, .. } if request_id == expected_request_id => {}
        _ => return Err(HttpRequestError::ResultBeforeCompletion),
    }
    if images.is_empty() {
        return Err(HttpRequestError::EmptyImages);
    }
    for image in images {
        if !is_fal_media_url(&image.url) {
            return Err(HttpRequestError::InvalidImageUrl);
        }
        if !matches!(image.content_type.as_str(), "image/jpeg" | "image/png") {
            return Err(HttpRequestError::InvalidImageContentType);
        }
        if image.width == 0 || image.height == 0 {
            return Err(HttpRequestError::InvalidImageDimensions);
        }
    }
    Ok(())
}

/// Constructs a canonical status URL instead of trusting supplier-returned URLs.
pub fn fal_status_request(
    api_key: &str,
    request_id: &ProviderRequestId,
) -> Result<HttpRequest, HttpRequestError> {
    fal_get_request(api_key, request_id, "status")
}

/// Constructs the canonical result URL for a previously captured request ID.
pub fn fal_result_request(
    api_key: &str,
    request_id: &ProviderRequestId,
    observation: &FalQueueObservation,
) -> Result<HttpRequest, HttpRequestError> {
    match observation {
        FalQueueObservation::Completed {
            request_id: completed_id,
            ..
        } if completed_id == request_id => {}
        _ => return Err(HttpRequestError::ResultBeforeCompletion),
    }
    fal_get_request(api_key, request_id, "response")
}

fn fal_get_request(
    api_key: &str,
    request_id: &ProviderRequestId,
    suffix: &str,
) -> Result<HttpRequest, HttpRequestError> {
    if api_key.is_empty() {
        return Err(HttpRequestError::EmptyCredential);
    }
    let suffix = if suffix.is_empty() {
        String::new()
    } else {
        format!("/{suffix}")
    };
    let url = format!(
        "{FAL_QUEUE_BASE}/requests/{}{suffix}",
        encode_path_segment(request_id.as_str())
    );
    Ok(HttpRequest {
        method: HttpMethod::Get,
        url,
        headers: vec![("Authorization", format!("Key {api_key}"))],
        body: Vec::new(),
    })
}

fn expected_fal_status_url(request_id: &ProviderRequestId) -> String {
    format!(
        "{FAL_QUEUE_BASE}/requests/{}/status",
        encode_path_segment(request_id.as_str())
    )
}

fn expected_fal_response_url(request_id: &ProviderRequestId) -> String {
    format!(
        "{FAL_QUEUE_BASE}/requests/{}/response",
        encode_path_segment(request_id.as_str())
    )
}

fn is_fal_media_url(url: &str) -> bool {
    let Some(host_and_path) = url.strip_prefix("https://") else {
        return false;
    };
    let Some((host, path)) = host_and_path.split_once('/') else {
        return false;
    };
    !host.is_empty()
        && host
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
        && (host == "fal.media" || host.ends_with(".fal.media"))
        && !path.is_empty()
}

fn nonempty_header(value: Option<&str>) -> Option<String> {
    value
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
}

pub(crate) fn encode_path_segment(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            use fmt::Write as _;
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

fn json_string(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len() + 2);
    encoded.push('"');
    for character in value.chars() {
        match character {
            '"' => encoded.push_str("\\\""),
            '\\' => encoded.push_str("\\\\"),
            '\u{08}' => encoded.push_str("\\b"),
            '\u{0C}' => encoded.push_str("\\f"),
            '\n' => encoded.push_str("\\n"),
            '\r' => encoded.push_str("\\r"),
            '\t' => encoded.push_str("\\t"),
            character if character <= '\u{1F}' => {
                use fmt::Write as _;
                let _ = write!(encoded, "\\u{:04x}", character as u32);
            }
            character => encoded.push(character),
        }
    }
    encoded.push('"');
    encoded
}
