//! Private API image backends: OpenAI, fal and BFL execution.
//!
//! This module owns the relocated supplier execution previously in
//! `df-providers::image_edit`, `df-providers::fal_image` and
//! `df-providers::bfl_image`. Old files stay untouched until the coordinator
//! migrates callers and removes the bypass. Nothing here is public outside
//! `df-imagery`: the later service exposes behavior, never these executors.
//!
//! What this module never does: retry, reword after refusal, reroute to
//! another supplier, log prompts or bytes, or record spend. One fal edit with
//! a transparent background is two separately metered stages; the caller sees
//! both receipts and never a single merged charge.

pub(crate) mod bfl;
pub(crate) mod codec;
mod failure;
pub(crate) mod fal;
pub(crate) use failure::{ApiExchange, ApiFailure, DeclineContract, SubmissionEvidence};
pub(crate) mod openai;

#[cfg(test)]
mod bfl_tests;
#[cfg(test)]
mod codec_tests;
#[cfg(test)]
mod fal_tests;
#[cfg(test)]
mod openai_tests;

use std::time::Duration;

/// Reused supplier prompt ceiling; kept as a single choke point.
pub(crate) use df_providers::IMAGE_EDIT_MAXIMUM_PROMPT_CHARACTERS;
/// Shared transport refusal; callers branch on variants, never on text.
pub(crate) use df_providers::NativeHttpRefusal;

/// Why a private API stage produced no image. Never carries a prompt, bytes
/// or supplier text; refusal and quota outcomes are terminal by contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ApiError {
    InvalidRequest,
    Refused,
    Unauthorized,
    Provider(u16),
    QuotaExhausted,
    RateLimited,
    Transport(NativeHttpRefusal),
    Output,
}

/// Default OpenAI edit models. Preserved constants; requests may also carry a
/// validated configured identifier for a schema-compatible model.
/// `NativeHttp` refuses a total timeout above this. A stage budget can be longer, because every
/// exchange is bounded by the transport and the budget is spent across exchanges.
pub(crate) const HTTP_TIMEOUT_CEILING: std::time::Duration = std::time::Duration::from_secs(180);
pub(crate) const GPT_25_FLARE_MODEL_ID: &str = "gpt-image-2.5-flare";
pub(crate) const GPT_25_SUNBURST_MODEL_ID: &str = "gpt-image-2.5-sunburst";
/// Longest configured model identifier accepted; bounds a nonsecret string.
pub(crate) const MAXIMUM_MODEL_ID_BYTES: usize = 128;

/// An edit model this backend calls. The two defaults keep their behavior;
/// `Custom` carries a validated operator-configured identifier for a
/// schema-compatible model. Capability validation stays at the call site.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ImageEditModel {
    Gpt25Flare,
    Gpt25Sunburst,
    Custom(String),
}

impl ImageEditModel {
    pub(crate) fn model_id(&self) -> &str {
        match self {
            Self::Gpt25Flare => GPT_25_FLARE_MODEL_ID,
            Self::Gpt25Sunburst => GPT_25_SUNBURST_MODEL_ID,
            Self::Custom(id) => id,
        }
    }

    pub(crate) fn from_model_id(id: &str) -> Option<Self> {
        match id {
            GPT_25_FLARE_MODEL_ID => Some(Self::Gpt25Flare),
            GPT_25_SUNBURST_MODEL_ID => Some(Self::Gpt25Sunburst),
            _ => None,
        }
    }

    pub(crate) fn parse_model_id(id: &str) -> Result<Self, ApiError> {
        if let Some(known) = Self::from_model_id(id) {
            return Ok(known);
        }
        check_model_id(id)?;
        Ok(Self::Custom(id.to_owned()))
    }
}

/// Validates a configured OpenAI-compatible model identifier: bounded,
/// printable, with namespace separators allowed in the JSON or multipart value.
/// Capability is validated by the caller, never inferred here.
pub(crate) fn check_model_id(id: &str) -> Result<(), ApiError> {
    if id.is_empty()
        || id.len() > MAXIMUM_MODEL_ID_BYTES
        || id.bytes().any(|byte| {
            !(0x21..=0x7e).contains(&byte)
                || matches!(byte, b'?' | b'#' | b'@' | b'\\' | b'"' | b' ')
        })
    {
        return Err(ApiError::InvalidRequest);
    }
    Ok(())
}

/// Validates a fal model slug or BFL route: bounded path-safe segments.
/// Slashes separate slug segments; `..`, query, fragment, userinfo and
/// control characters are rejected.
pub(crate) fn check_model_slug(slug: &str) -> Result<(), ApiError> {
    if slug.is_empty()
        || slug.len() > 256
        || slug.starts_with('/')
        || slug.ends_with('/')
        || slug.contains("..")
        || slug.bytes().any(|byte| {
            !(0x21..=0x7e).contains(&byte)
                || matches!(
                    byte,
                    b'?' | b'#'
                        | b'@'
                        | b'\\'
                        | b'"'
                        | b' '
                        | b':'
                        | b'<'
                        | b'>'
                        | b'{'
                        | b'}'
                        | b'|'
                        | b'^'
                        | b'`'
                )
        })
        || !slug
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'/'))
    {
        return Err(ApiError::InvalidRequest);
    }
    Ok(())
}

/// Validates an explicit operator endpoint URL. Accepts `https://` with a
/// structurally valid host and optional port plus an origin-relative path,
/// and `http://` only for loopback destinations. Rejects userinfo (`@`),
/// query, fragments, encoded separators/control bytes and dot-segment traversal.
/// RFC path characters and safe percent escapes are preserved. Returns whether
/// the transport may use plain HTTP (loopback only). One validated parser
/// backs every backend so safety never differs by caller.
/// Generation origins receive the account credential. Plain HTTP is loopback-only, and
/// HTTPS is accepted only for the named production host, so an operator override cannot
/// redirect a key to another destination. Structural validation is shared with
/// `check_endpoint_url`. Analysis routes keep their own, broader endpoint rules.
pub(crate) fn check_credentialed_origin(
    url: &str,
    production_host: &str,
) -> Result<bool, ApiError> {
    if check_endpoint_url(url)? {
        return Ok(true);
    }
    let parsed = url::Url::parse(url).map_err(|_| ApiError::InvalidRequest)?;
    let pinned = parsed
        .host_str()
        .is_some_and(|host| host.eq_ignore_ascii_case(production_host));
    if pinned && parsed.port_or_known_default() == Some(443) {
        Ok(false)
    } else {
        Err(ApiError::InvalidRequest)
    }
}

pub(crate) fn check_endpoint_url(url: &str) -> Result<bool, ApiError> {
    if url.is_empty() || url.len() > 8192 {
        return Err(ApiError::InvalidRequest);
    }
    if url.bytes().any(|byte| !(0x21..0x7f).contains(&byte)) {
        return Err(ApiError::InvalidRequest);
    }
    if url.contains(['?', '#', '\\', '"', '<', '>', '|', '`']) {
        return Err(ApiError::InvalidRequest);
    }
    let (scheme, rest) = url.split_once("://").ok_or(ApiError::InvalidRequest)?;
    if scheme.eq_ignore_ascii_case("https") {
        let (authority, path) = split_authority_path(rest)?;
        validate_authority(authority, false)?;
        validate_endpoint_path(path)?;
        Ok(false)
    } else if scheme.eq_ignore_ascii_case("http") {
        let (authority, path) = split_authority_path(rest)?;
        validate_authority(authority, true)?;
        validate_endpoint_path(path)?;
        Ok(true)
    } else {
        Err(ApiError::InvalidRequest)
    }
}

fn split_authority_path(rest: &str) -> Result<(&str, &str), ApiError> {
    if rest.is_empty() {
        return Err(ApiError::InvalidRequest);
    }
    match rest.find('/') {
        Some(index) => Ok((&rest[..index], &rest[index..])),
        None => Ok((rest, "")),
    }
}

fn validate_endpoint_path(path: &str) -> Result<(), ApiError> {
    if !path.is_empty() && !path.starts_with('/') {
        return Err(ApiError::InvalidRequest);
    }
    for segment in path.split('/') {
        let mut bytes = segment.bytes();
        let mut decoded_length = 0;
        let mut contains_only_dots = true;
        while let Some(byte) = bytes.next() {
            let decoded = if byte == b'%' {
                let high = bytes
                    .next()
                    .and_then(hex_digit)
                    .ok_or(ApiError::InvalidRequest)?;
                let low = bytes
                    .next()
                    .and_then(hex_digit)
                    .ok_or(ApiError::InvalidRequest)?;
                let decoded = high * 16 + low;
                if decoded.is_ascii_control() || matches!(decoded, b'/' | b'\\' | b'%') {
                    return Err(ApiError::InvalidRequest);
                }
                decoded
            } else {
                if !byte.is_ascii_alphanumeric()
                    && !matches!(
                        byte,
                        b'-' | b'.'
                            | b'_'
                            | b'~'
                            | b'!'
                            | b'$'
                            | b'&'
                            | b'\''
                            | b'('
                            | b')'
                            | b'*'
                            | b'+'
                            | b','
                            | b';'
                            | b'='
                            | b':'
                            | b'@'
                    )
                {
                    return Err(ApiError::InvalidRequest);
                }
                byte
            };
            decoded_length += 1;
            contains_only_dots &= decoded == b'.';
        }
        if contains_only_dots && matches!(decoded_length, 1 | 2) {
            return Err(ApiError::InvalidRequest);
        }
    }
    Ok(())
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn validate_authority(authority: &str, require_loopback: bool) -> Result<(), ApiError> {
    if authority.is_empty() || authority.len() > 253 {
        return Err(ApiError::InvalidRequest);
    }
    if let Some(bracketed) = authority.strip_prefix('[') {
        let end = bracketed.find(']').ok_or(ApiError::InvalidRequest)?;
        let inside = &bracketed[..end];
        let remainder = &bracketed[end + 1..];
        if inside.is_empty()
            || !inside
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() || matches!(byte, b':' | b'.'))
        {
            return Err(ApiError::InvalidRequest);
        }
        let address: std::net::IpAddr = inside.parse().map_err(|_| ApiError::InvalidRequest)?;
        if require_loopback && !address.is_loopback() {
            return Err(ApiError::InvalidRequest);
        }
        if remainder.is_empty() {
            return Ok(());
        }
        let port = remainder
            .strip_prefix(':')
            .ok_or(ApiError::InvalidRequest)?;
        validate_port(port)?;
        return Ok(());
    }
    if authority.contains('[') || authority.contains(']') {
        return Err(ApiError::InvalidRequest);
    }
    let (host, port) = match authority.rfind(':') {
        Some(index) => (&authority[..index], Some(&authority[index + 1..])),
        None => (authority, None),
    };
    if let Some(port) = port {
        validate_port(port)?;
    }
    if host.is_empty() || host.contains(':') {
        return Err(ApiError::InvalidRequest);
    }
    if host.eq_ignore_ascii_case("localhost") {
        return Ok(());
    }
    if let Ok(address) = host.parse::<std::net::IpAddr>() {
        if require_loopback && !address.is_loopback() {
            return Err(ApiError::InvalidRequest);
        }
        return Ok(());
    }
    if require_loopback {
        return Err(ApiError::InvalidRequest);
    }
    validate_dns_host(host)
}

fn validate_port(port: &str) -> Result<(), ApiError> {
    if port.is_empty() || port.len() > 5 || !port.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(ApiError::InvalidRequest);
    }
    match port.parse::<u16>() {
        Ok(0) | Err(_) => Err(ApiError::InvalidRequest),
        Ok(_) => Ok(()),
    }
}

fn validate_dns_host(host: &str) -> Result<(), ApiError> {
    if host.len() > 253 {
        return Err(ApiError::InvalidRequest);
    }
    let mut labels = 0;
    for label in host.split('.') {
        labels += 1;
        if label.is_empty()
            || label.len() > 63
            || !label
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            || label.starts_with('-')
            || label.ends_with('-')
        {
            return Err(ApiError::InvalidRequest);
        }
    }
    if labels == 0 {
        return Err(ApiError::InvalidRequest);
    }
    Ok(())
}

/// How hard the model works; it sets both cost and latency.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ImageQuality {
    Low,
    Medium,
    High,
}

impl ImageQuality {
    pub(crate) const fn wire(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

/// Container of an image: references arrive as stored, results leave in one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ImageFormat {
    Png,
    Jpeg,
    Webp,
}

impl ImageFormat {
    pub(crate) const fn mime(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Webp => "image/webp",
        }
    }

    pub(crate) const fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::Webp => "webp",
        }
    }

    pub(crate) const fn wire(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpeg",
            Self::Webp => "webp",
        }
    }
}

/// Whether the result carries an alpha channel. Transparent is always PNG.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ImageBackground {
    Opaque,
    Transparent,
}

/// Supplier moderation strictness, chosen once per request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ImageModeration {
    Auto,
    Low,
}

/// One reference image of an edit.
#[derive(Clone, Copy)]
pub(crate) struct ReferenceImage<'a> {
    pub(crate) format: ImageFormat,
    pub(crate) bytes: &'a [u8],
}

/// One edit: faithful references plus the instruction changing them.
pub(crate) struct ImageEdit<'a> {
    pub(crate) model: ImageEditModel,
    pub(crate) prompt: &'a str,
    pub(crate) references: &'a [ReferenceImage<'a>],
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) quality: ImageQuality,
    pub(crate) background: ImageBackground,
    pub(crate) output: ImageFormat,
    pub(crate) moderation: ImageModeration,
}

/// Raw supplier usage units for one OpenAI call. Carried unchanged to the
/// existing cost owner; this crate keeps no dollar ledger or rate table.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ImageUsage {
    pub(crate) text_input_tokens: u64,
    pub(crate) image_input_tokens: u64,
    pub(crate) output_tokens: u64,
}

/// A complete, structurally checked result with actual bytes and usage.
#[derive(Clone, Eq, PartialEq)]
pub(crate) struct GeneratedImage {
    pub(crate) bytes: Vec<u8>,
    pub(crate) format: ImageFormat,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) has_alpha_channel: bool,
    pub(crate) usage: ImageUsage,
}

impl std::fmt::Debug for GeneratedImage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GeneratedImage")
            .field("byte_length", &self.bytes.len())
            .field("width", &self.width)
            .field("height", &self.height)
            .field("has_alpha_channel", &self.has_alpha_channel)
            .finish()
    }
}

/// Injectable operational bounds. Defaults preserve current compatibility
/// behavior; composition supplies the snapshot, never ambient state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct OperationalBounds {
    pub(crate) openai_max_request_bytes: usize,
    pub(crate) openai_max_response_bytes: usize,
    pub(crate) fal_max_request_bytes: usize,
    pub(crate) fal_max_response_bytes: usize,
    pub(crate) fal_max_image_bytes: usize,
    pub(crate) bfl_max_delivery_bytes: usize,
    pub(crate) poll_interval: Duration,
}

impl OperationalBounds {
    pub(crate) fn compatibility_defaults() -> Self {
        Self {
            openai_max_request_bytes: 12 * 1024 * 1024,
            openai_max_response_bytes: 16 * 1024 * 1024,
            fal_max_request_bytes: 16 * 1024 * 1024,
            fal_max_response_bytes: 16 * 1024 * 1024,
            fal_max_image_bytes: 11 * 1024 * 1024,
            bfl_max_delivery_bytes: 6 * 1024 * 1024,
            poll_interval: Duration::from_millis(500),
        }
    }

    pub(crate) fn validate(self) -> Result<(), ApiError> {
        if self.openai_max_request_bytes == 0
            || self.openai_max_response_bytes == 0
            || self.fal_max_request_bytes == 0
            || self.fal_max_response_bytes == 0
            || self.fal_max_image_bytes == 0
            || self.bfl_max_delivery_bytes == 0
            || self.poll_interval.is_zero()
        {
            return Err(ApiError::InvalidRequest);
        }
        Ok(())
    }
}

/// Shared credential gate: printable ASCII, 1..=4096 bytes. Secrets never
/// reach diagnostics; only the refusal outcome survives.
pub(crate) fn check_credential(credential: &str) -> Result<(), ApiError> {
    if credential.is_empty()
        || credential.len() > 4096
        || !credential.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
    {
        return Err(ApiError::InvalidRequest);
    }
    Ok(())
}

#[cfg(test)]
mod credentialed_origin_tests {
    use super::check_credentialed_origin;

    #[test]
    fn pinned_production_host_is_accepted_over_https() {
        assert!(matches!(
            check_credentialed_origin("https://api.openai.com/v1/images/edits", "api.openai.com"),
            Ok(false)
        ));
    }

    #[test]
    fn other_https_hosts_are_refused_even_when_structurally_valid() {
        for origin in [
            "https://attacker.example/v1/images/edits",
            "https://api.openai.com.evil.example/v1/images/edits",
            "https://api.openai.com:8443/v1/images/edits",
        ] {
            assert!(
                check_credentialed_origin(origin, "api.openai.com").is_err(),
                "{origin} must not receive the credential"
            );
        }
    }

    #[test]
    fn plain_http_is_limited_to_loopback() {
        assert!(matches!(
            check_credentialed_origin("http://127.0.0.1:9/v1/images/edits", "api.openai.com"),
            Ok(true)
        ));
        assert!(
            check_credentialed_origin("http://api.openai.com/v1/images/edits", "api.openai.com")
                .is_err()
        );
    }
}
