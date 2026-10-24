//! Bounded inspection policy for completed FAL image outputs.
//!
//! A plan is not network permission. This module performs no DNS lookup, HTTP
//! request, redirect handling, byte retrieval, decoding, or publication. The
//! later executor must enforce the declared transport and isolation controls.

use std::{
    fmt,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
};

use df_types::OperationId;

use crate::{
    CandidateRouteId, FalQueueObservation, ProviderAttemptIdentity, ProviderImageMetadata,
    ProviderRequestId, validate_fal_images,
};

const MAX_ENCODED_BYTES: u64 = 16 * 1024 * 1024;
const MAX_HEADER_BYTES: usize = 16 * 1024;
const MAX_PIXELS: u64 = 16_777_216;
const MAX_SIDE: u32 = 8_192;
const MAX_RGBA_BYTES: u64 = 64 * 1024 * 1024;
const MAX_DECODE_RSS_BYTES: u64 = 128 * 1024 * 1024;
const MAX_DECODE_WALL_SECONDS: u8 = 5;
const MAX_DNS_ANSWERS: usize = 16;

/// Stable, non-sensitive refusal classifications for a future bounded executor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputFetchRefusal {
    RequestNotCompleted,
    RequestIdentityMismatch,
    OutputUrlRefused,
    DnsTargetRefused,
    RedirectRefused,
    ProxyRefused,
    HttpStatusRefused,
    DeadlineExceeded,
    BodyTooLarge,
    MimeMismatch,
    DimensionsMismatch,
    DecodeRejected,
    DecoderUnavailable,
    Cancelled,
    TransportOutcomeUnknown,
    HeadersTooLarge,
}

/// Network and resource controls a future fetch executor must enforce.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FetchRequirements {
    pub https_only: bool,
    pub redirects_disabled: bool,
    pub proxy_disabled: bool,
    pub connect_timeout_seconds: u8,
    pub total_timeout_seconds: u8,
    pub max_header_bytes: usize,
    pub max_encoded_bytes: u64,
}

/// Mandatory one-shot decoder isolation contract; no caller assertion can grant it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DecoderRequirements {
    pub network: bool,
    pub credentials: bool,
    pub inherited_environment: bool,
    pub writable_filesystem: bool,
    pub max_pixels: u64,
    pub max_side: u32,
    pub rgba_bytes: u64,
    pub rss_bytes: u64,
    pub wall_seconds: u8,
}

/// The native decoder is unavailable until OS enforcement is implemented and evidenced.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecoderPolicy {
    DecoderUnavailable(DecoderRequirements),
}

/// A pure finite inspection of supplied DNS candidates, not a DNS lookup.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DnsInspection {
    candidates: Vec<IpAddr>,
}

impl DnsInspection {
    pub fn candidates(&self) -> &[IpAddr] {
        &self.candidates
    }
}

/// Inspection-only candidate for the later bounded fetch executor.
#[derive(Clone, Eq, PartialEq)]
pub struct OutputFetchPlan {
    operation: OperationId,
    route: CandidateRouteId,
    supplier_request_id: ProviderRequestId,
    locator: String,
    content_type: String,
    width: u32,
    height: u32,
    fetch: FetchRequirements,
    decoder: DecoderPolicy,
}

/// Bounded response facts supplied by a future transport owner for pure policy inspection.
/// Durations are whole seconds measured by that owner. These facts alone never
/// establish provenance, byte integrity, proxy configuration, or fetch completion.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FetchResponseFacts<'a> {
    pub status: u16,
    pub redirected: bool,
    pub used_proxy: bool,
    pub cancelled: bool,
    pub header_bytes: usize,
    pub content_type: &'a str,
    pub body_bytes: u64,
    pub body_complete: bool,
    pub connect_elapsed_seconds: u64,
    pub elapsed_seconds: u64,
}

/// Decode facts from a future isolated helper; inspection is not an asset reference.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DecodedImageFacts<'a> {
    pub format: &'a str,
    pub width: u32,
    pub height: u32,
    pub input_bytes: u64,
    pub consumed_bytes: u64,
    pub frame_count: u32,
    pub animated: bool,
    pub unsupported_metadata: bool,
    pub external_references: bool,
}

impl fmt::Debug for OutputFetchPlan {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OutputFetchPlan")
            .field("operation", &self.operation)
            .field("route", &self.route)
            .field("supplier_request_id", &"[REDACTED]")
            .field("locator", &"[REDACTED]")
            .field("content_type", &self.content_type)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("fetch", &self.fetch)
            .field("decoder", &self.decoder)
            .finish()
    }
}

impl OutputFetchPlan {
    pub const fn operation(&self) -> OperationId {
        self.operation
    }
    pub const fn route(&self) -> CandidateRouteId {
        self.route
    }
    pub fn supplier_request_id(&self) -> &ProviderRequestId {
        &self.supplier_request_id
    }
    pub fn locator(&self) -> &str {
        &self.locator
    }
    pub fn content_type(&self) -> &str {
        &self.content_type
    }
    pub const fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }
    pub const fn fetch_requirements(&self) -> FetchRequirements {
        self.fetch
    }
    pub const fn decoder_policy(&self) -> DecoderPolicy {
        self.decoder
    }
}

/// Inspect a completed image tuple under exact attempt and locator policy.
///
/// `maximum_bytes` is a requested cap and is clamped to the D04 hard ceiling.
/// It never authorizes a socket or decoder. The operation, route, and supplier
/// ID must match the persisted D03 attempt before any plan is returned.
pub fn prepare_output_fetch_plan(
    attempt: &ProviderAttemptIdentity,
    expected_operation: OperationId,
    expected_request_id: &ProviderRequestId,
    observation: &FalQueueObservation,
    image: &ProviderImageMetadata,
    maximum_bytes: u64,
) -> Result<OutputFetchPlan, OutputFetchRefusal> {
    if attempt.operation() != expected_operation
        || attempt.route() != CandidateRouteId::FalFluxSchnellDisposableImage
        || attempt.supplier_request_id() != Some(expected_request_id)
    {
        return Err(OutputFetchRefusal::RequestIdentityMismatch);
    }
    if !matches!(observation, FalQueueObservation::Completed { request_id, .. } if request_id == expected_request_id)
    {
        return Err(OutputFetchRefusal::RequestNotCompleted);
    }
    validate_fal_images(
        observation,
        expected_request_id,
        std::slice::from_ref(image),
    )
    .map_err(|error| match error {
        crate::HttpRequestError::InvalidImageContentType => OutputFetchRefusal::MimeMismatch,
        crate::HttpRequestError::InvalidImageDimensions => OutputFetchRefusal::DimensionsMismatch,
        crate::HttpRequestError::ResultBeforeCompletion => OutputFetchRefusal::RequestNotCompleted,
        _ => OutputFetchRefusal::OutputUrlRefused,
    })?;
    if !strict_fal_locator(&image.url) {
        return Err(OutputFetchRefusal::OutputUrlRefused);
    }
    let pixels = u64::from(image.width)
        .checked_mul(u64::from(image.height))
        .ok_or(OutputFetchRefusal::DimensionsMismatch)?;
    let rgba_bytes = pixels
        .checked_mul(4)
        .ok_or(OutputFetchRefusal::DimensionsMismatch)?;
    if image.width > MAX_SIDE
        || image.height > MAX_SIDE
        || pixels > MAX_PIXELS
        || rgba_bytes > MAX_RGBA_BYTES
    {
        return Err(OutputFetchRefusal::DimensionsMismatch);
    }
    if maximum_bytes == 0 {
        return Err(OutputFetchRefusal::BodyTooLarge);
    }

    Ok(OutputFetchPlan {
        operation: expected_operation,
        route: attempt.route(),
        supplier_request_id: expected_request_id.clone(),
        locator: image.url.clone(),
        content_type: image.content_type.clone(),
        width: image.width,
        height: image.height,
        fetch: FetchRequirements {
            https_only: true,
            redirects_disabled: true,
            proxy_disabled: true,
            connect_timeout_seconds: 2,
            total_timeout_seconds: 15,
            max_header_bytes: MAX_HEADER_BYTES,
            max_encoded_bytes: maximum_bytes.min(MAX_ENCODED_BYTES),
        },
        decoder: DecoderPolicy::DecoderUnavailable(DecoderRequirements {
            network: false,
            credentials: false,
            inherited_environment: false,
            writable_filesystem: false,
            max_pixels: MAX_PIXELS,
            max_side: MAX_SIDE,
            rgba_bytes: MAX_RGBA_BYTES,
            rss_bytes: MAX_DECODE_RSS_BYTES,
            wall_seconds: MAX_DECODE_WALL_SECONDS,
        }),
    })
}

/// Inspect response facts with the plan's strict transport bounds. No I/O occurs.
pub fn inspect_fetch_response(
    plan: &OutputFetchPlan,
    facts: FetchResponseFacts<'_>,
) -> Result<(), OutputFetchRefusal> {
    if facts.cancelled {
        return Err(OutputFetchRefusal::Cancelled);
    }
    if facts.used_proxy {
        return Err(OutputFetchRefusal::ProxyRefused);
    }
    if facts.redirected || (300..400).contains(&facts.status) {
        return Err(OutputFetchRefusal::RedirectRefused);
    }
    if facts.status != 200 {
        return Err(OutputFetchRefusal::HttpStatusRefused);
    }
    if facts.connect_elapsed_seconds > facts.elapsed_seconds {
        return Err(OutputFetchRefusal::TransportOutcomeUnknown);
    }
    if facts.connect_elapsed_seconds > u64::from(plan.fetch.connect_timeout_seconds)
        || facts.elapsed_seconds > u64::from(plan.fetch.total_timeout_seconds)
    {
        return Err(OutputFetchRefusal::DeadlineExceeded);
    }
    if facts.header_bytes > plan.fetch.max_header_bytes {
        return Err(OutputFetchRefusal::HeadersTooLarge);
    }
    if facts.body_bytes == 0 || facts.body_bytes > plan.fetch.max_encoded_bytes {
        return Err(OutputFetchRefusal::BodyTooLarge);
    }
    if !facts.body_complete {
        return Err(OutputFetchRefusal::TransportOutcomeUnknown);
    }
    if facts.content_type != plan.content_type {
        return Err(OutputFetchRefusal::MimeMismatch);
    }
    Ok(())
}

/// Inspect metadata and completeness facts from an isolated decoder result.
///
/// This validates claims only. The function cannot prove that a helper was
/// isolated, that input bytes came from the planned locator, or that pixels are
/// safe to publish. The current operational decoder always refuses below.
pub fn inspect_decoded_image(
    plan: &OutputFetchPlan,
    facts: DecodedImageFacts<'_>,
) -> Result<(), OutputFetchRefusal> {
    let expected_format = match plan.content_type.as_str() {
        "image/jpeg" => "image/jpeg",
        "image/png" => "image/png",
        _ => return Err(OutputFetchRefusal::MimeMismatch),
    };
    if facts.format != expected_format {
        return Err(OutputFetchRefusal::MimeMismatch);
    }
    if (facts.width, facts.height) != (plan.width, plan.height) {
        return Err(OutputFetchRefusal::DimensionsMismatch);
    }
    let pixels = u64::from(facts.width)
        .checked_mul(u64::from(facts.height))
        .ok_or(OutputFetchRefusal::DecodeRejected)?;
    let rgba = pixels
        .checked_mul(4)
        .ok_or(OutputFetchRefusal::DecodeRejected)?;
    if pixels > MAX_PIXELS
        || facts.width > MAX_SIDE
        || facts.height > MAX_SIDE
        || rgba > MAX_RGBA_BYTES
        || facts.frame_count != 1
        || facts.animated
        || facts.unsupported_metadata
        || facts.external_references
        || facts.input_bytes == 0
        || facts.consumed_bytes != facts.input_bytes
    {
        return Err(OutputFetchRefusal::DecodeRejected);
    }
    if facts.input_bytes > plan.fetch.max_encoded_bytes {
        return Err(OutputFetchRefusal::BodyTooLarge);
    }
    Ok(())
}

/// Operational acquisition entrypoint, deliberately fail-closed until a bounded
/// HTTP client with DNS pinning and redirect/proxy controls exists.
pub fn fetch_output(_plan: &OutputFetchPlan) -> Result<(), OutputFetchRefusal> {
    Err(OutputFetchRefusal::TransportOutcomeUnknown)
}

/// Operational decoder entrypoint, deliberately fail-closed until an enforced
/// one-shot OS sandbox and resource limits are available on this target.
pub fn decode_output(_plan: &OutputFetchPlan) -> Result<(), OutputFetchRefusal> {
    Err(OutputFetchRefusal::DecoderUnavailable)
}

/// Validate already-resolved DNS candidates without performing resolution.
/// Empty, excessive, or mixed/private answers fail closed.
pub fn inspect_dns_answers(candidates: &[IpAddr]) -> Result<DnsInspection, OutputFetchRefusal> {
    if candidates.is_empty()
        || candidates.len() > MAX_DNS_ANSWERS
        || candidates.iter().any(|address| !is_public(*address))
    {
        return Err(OutputFetchRefusal::DnsTargetRefused);
    }
    Ok(DnsInspection {
        candidates: candidates.to_vec(),
    })
}

pub(crate) fn strict_fal_locator(url: &str) -> bool {
    let Some(authority_and_path) = url.strip_prefix("https://") else {
        return false;
    };
    if url.contains(['?', '#', '\\', '\0']) || !url.is_ascii() {
        return false;
    }
    let Some((authority, path)) = authority_and_path.split_once('/') else {
        return false;
    };
    if authority.is_empty() || path.is_empty() || authority.contains(['@', ':']) {
        return false;
    }
    let host = authority.to_ascii_lowercase();
    if authority != host
        || !(host == "fal.media" || host.strip_suffix(".fal.media").is_some_and(valid_subdomain))
    {
        return false;
    }
    if host.parse::<IpAddr>().is_ok() {
        return false;
    }
    if path
        .bytes()
        .any(|byte| byte.is_ascii_control() || byte == b' ')
    {
        return false;
    }
    let bytes = path.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len()
                || !bytes[index + 1].is_ascii_hexdigit()
                || !bytes[index + 2].is_ascii_hexdigit()
            {
                return false;
            }
            let decoded = u8::from_str_radix(&path[index + 1..index + 3], 16).ok();
            if matches!(decoded, Some(b'/' | b'\\' | b'.' | b'?' | b'#' | 0)) {
                return false;
            }
            index += 3;
        } else {
            index += 1;
        }
    }
    !path
        .split('/')
        .any(|segment| segment == "." || segment == "..")
}

fn valid_subdomain(value: &str) -> bool {
    !value.is_empty()
        && value.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label.as_bytes()[0].is_ascii_alphanumeric()
                && label.as_bytes()[label.len() - 1].is_ascii_alphanumeric()
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
}

fn is_public(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(ip) => public_v4(ip),
        IpAddr::V6(ip) => public_v6(ip),
    }
}

fn public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_unspecified()
        || a == 0
        || a >= 224
        || (a == 100 && (64..=127).contains(&b))
        || (a == 169 && b == 254)
        || (a == 192 && b == 0 && c == 0)
        || (a == 192 && b == 0 && c == 2)
        || (a == 198 && (b == 18 || b == 19 || b == 51 && c == 100))
        || (a == 203 && b == 0 && c == 113)
        || (a == 192 && b == 88 && c == 99))
}

fn public_v6(ip: Ipv6Addr) -> bool {
    if let Some(mapped) = ip.to_ipv4_mapped() {
        return public_v4(mapped);
    }
    let segments = ip.segments();
    !(ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_unique_local()
        || ip.is_unicast_link_local()
        || segments[0] & 0xffc0 == 0xfe80
        || segments[0] & 0xff00 == 0xff00
        // Global unicast is 2000::/3; /7 incorrectly rejects allocated 2600::/12.
        // https://www.iana.org/assignments/ipv6-unicast-address-assignments/
        || segments[0] & 0xe000 != 0x2000
        || segments[0] == 0x2001 && (segments[1] <= 0x01ff || segments[1] == 0x0db8)
        || segments[0] == 0x2002
        || segments[0] == 0x3fff)
}

#[cfg(test)]
mod dns_global_unicast_tests {
    use super::*;
    #[test]
    fn bfl_azure_dual_stack_public_answers_are_admitted() {
        let answers = [
            "150.171.109.117".parse().unwrap(),
            "2603:1061:14:46::1".parse().unwrap(),
        ];
        assert!(inspect_dns_answers(&answers).is_ok());
        for address in [
            "2606:4700:4700::1111",
            "2a00:1450:4001:800::200e",
            "2404:6800:4001::1",
        ] {
            assert!(inspect_dns_answers(&[address.parse().unwrap()]).is_ok());
        }
    }
    #[test]
    fn mixed_private_and_reserved_dns_answers_still_fail_closed() {
        let public = "2603:1061:14:46::1".parse::<IpAddr>().unwrap();
        for denied in [
            "::",
            "::1",
            "fc00::1",
            "fd00::1",
            "fe80::1",
            "ff02::1",
            "2001:db8::1",
            "2001::1",
            "2002:0808:0808::1",
            "3fff::1",
            "4000::1",
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
            "10.0.0.1",
            "169.254.169.254",
        ] {
            let denied = denied.parse().unwrap();
            assert!(inspect_dns_answers(&[denied]).is_err());
            assert!(inspect_dns_answers(&[public, denied]).is_err());
        }
        assert!(inspect_dns_answers(&[]).is_err());
    }
}
