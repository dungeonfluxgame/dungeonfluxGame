//! Credential-free bounded native media acquisition from explicit socket pins.

use std::{
    future::Future,
    net::{IpAddr, SocketAddr},
    time::Duration,
};

use crate::native_http::{
    ExchangeRecord, ExchangeTrace, MAXIMUM_BODY_BYTES, NativeHttp, SendState,
    classify_transport_error, exchange_host, hardened_client_builder, header_block_bytes,
    is_loopback_socket, read_bounded_body,
};
use crate::output_fetch::{MAXIMUM_LOCATOR_BYTES, strict_fal_locator};
use crate::{NativeHttpCause, NativeHttpLimits, ProviderName, inspect_dns_answers};

/// The largest response header block a media fetcher accepts, in bytes.
pub(crate) const MAXIMUM_MEDIA_HEADER_BYTES: usize = 16_384;
/// The most pins one fetcher holds, and the most addresses across them (a DNS answer bound).
const MAXIMUM_PINNED: usize = 16;
/// The one content type a BFL delivery is asked for: either still format it may answer with.
const BFL_IMAGE_TYPES: &str = "image/png,image/jpeg";

#[derive(Clone, Eq, PartialEq)]
pub struct FalCdnPin {
    host: String,
    addresses: Vec<IpAddr>,
}

impl std::fmt::Debug for FalCdnPin {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("FalCdnPin").finish_non_exhaustive()
    }
}

/// Why a fal CDN pin was refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FalCdnPinError {
    /// The host is not a fal media host.
    InvalidHost,
    /// An address is not public, repeats, or the answer holds too many.
    InvalidAddresses,
}

impl std::fmt::Display for FalCdnPinError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidHost => "the pin host is not a fal media host",
            Self::InvalidAddresses => "the pin addresses are not distinct public addresses",
        })
    }
}

impl std::error::Error for FalCdnPinError {}

impl FalCdnPin {
    /// Public, finite DNS observations are caller-supplied. No resolver fallback is enabled.
    pub fn new(host: &str, addresses: &[IpAddr]) -> Result<Self, FalCdnPinError> {
        if !strict_fal_locator(&format!("https://{host}/pin")) {
            return Err(FalCdnPinError::InvalidHost);
        }
        let inspected =
            inspect_dns_answers(addresses).map_err(|_| FalCdnPinError::InvalidAddresses)?;
        if addresses.iter().enumerate().any(|(i, value)| {
            addresses
                .get(..i)
                .is_some_and(|earlier| earlier.contains(value))
        }) {
            return Err(FalCdnPinError::InvalidAddresses);
        }
        Ok(Self {
            host: host.to_owned(),
            addresses: inspected.candidates().to_vec(),
        })
    }
}

/// Bounded keep-alive for the shared media clients: a few idle sockets per host, released after
/// a short quiet period, so a long-lived owner cannot accumulate sockets.
const MEDIA_POOL_IDLE_PER_HOST: usize = 4;
const MEDIA_POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) struct NativeMediaFetch {
    limits: NativeHttpLimits,
    fixture: Option<SocketAddr>,
    bfl: bool,
    observed_status: std::sync::atomic::AtomicU16,
    /// Loopback fixture transport, present exactly when `fixture` is.
    fixture_client: Option<reqwest::Client>,
    /// One entry per validated pin. The pin and the client that resolves only its host are stored
    /// together, so a lookup by host cannot pair a client with another host's addresses.
    pinned: Vec<PinnedTransport>,
}

/// A validated pin with its pooled client. Connections stay alive across the files of one
/// response, so each file no longer pays for a new client, TLS session or pool.
struct PinnedTransport {
    pin: FalCdnPin,
    client: reqwest::Client,
}

fn build_pinned(
    limits: NativeHttpLimits,
    pins: Vec<FalCdnPin>,
) -> Result<Vec<PinnedTransport>, NativeHttpCause> {
    pins.into_iter()
        .map(|pin| {
            let addresses = pinned_socket_addresses(&pin);
            let client = media_client(limits, true, Some((&pin.host, &addresses)))?;
            Ok(PinnedTransport { pin, client })
        })
        .collect()
}

fn pinned_socket_addresses(pin: &FalCdnPin) -> Vec<SocketAddr> {
    pin.addresses
        .iter()
        .map(|address| SocketAddr::new(*address, 443))
        .collect()
}

fn media_client(
    limits: NativeHttpLimits,
    https_only: bool,
    resolve: Option<(&str, &[SocketAddr])>,
) -> Result<reqwest::Client, NativeHttpCause> {
    let mut builder = hardened_client_builder(&limits, https_only)
        .pool_max_idle_per_host(MEDIA_POOL_IDLE_PER_HOST)
        .pool_idle_timeout(MEDIA_POOL_IDLE_TIMEOUT);
    if let Some((host, addresses)) = resolve {
        builder = builder.resolve_to_addrs(host, addresses);
    }
    builder.build().map_err(|_| NativeHttpCause::Unknown)
}

impl NativeMediaFetch {
    pub(crate) fn new(
        limits: NativeHttpLimits,
        pins: Vec<FalCdnPin>,
        fixture: Option<SocketAddr>,
    ) -> Result<Self, NativeHttpCause> {
        if limits.maximum_header_bytes > MAXIMUM_MEDIA_HEADER_BYTES
            || pins.len() > MAXIMUM_PINNED
            || pins.iter().map(|pin| pin.addresses.len()).sum::<usize>() > MAXIMUM_PINNED
            || pins.iter().enumerate().any(|(i, pin)| {
                pins.get(..i)
                    .is_some_and(|earlier| earlier.iter().any(|old| old.host == pin.host))
            })
            || fixture.is_some_and(|address| !is_loopback_socket(address))
            || (fixture.is_none() && pins.is_empty())
        {
            return Err(NativeHttpCause::InvalidLimits);
        }
        // Reuse the validated finite native transport bounds without any network operation.
        NativeHttp::new(limits, fixture.is_some()).map_err(|error| error.cause)?;
        let fixture_client = match fixture {
            Some(_) => Some(media_client(limits, false, None)?),
            None => None,
        };
        // The explicit fixture transport never consults the pins, so none are retained for it.
        let pinned = if fixture.is_some() {
            Vec::new()
        } else {
            build_pinned(limits, pins)?
        };
        Ok(Self {
            limits,
            fixture,
            bfl: false,
            observed_status: std::sync::atomic::AtomicU16::new(0),
            fixture_client,
            pinned,
        })
    }

    pub(crate) fn observed_status(&self) -> Option<u16> {
        match self
            .observed_status
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            0 => None,
            value => Some(value),
        }
    }
    /// Resolves an allowlisted BFL delivery URL once and pins only public addresses.
    pub(crate) async fn bfl(
        limits: NativeHttpLimits,
        locator: &str,
        fixture: Option<SocketAddr>,
    ) -> Result<Self, BflDeliveryCause> {
        if let Some(fixture_address) = fixture {
            if !is_loopback_socket(fixture_address) {
                return Err(BflDeliveryCause::FixtureInvalid);
            }
            if !(strict_bfl_delivery(locator) || is_fixture_locator(fixture_address, locator)) {
                return Err(BflDeliveryCause::UrlContract);
            }
        } else if !strict_bfl_delivery(locator) {
            return Err(BflDeliveryCause::UrlContract);
        }
        let url = reqwest::Url::parse(locator).map_err(|_| BflDeliveryCause::UrlParse)?;
        let host = url.host_str().ok_or(BflDeliveryCause::MissingHost)?;
        // Validate transport bounds before DNS so unsupported limits never cause network I/O.
        NativeHttp::new(limits, fixture.is_some())
            .map_err(|error| BflDeliveryCause::TransportConfiguration(error.cause))?;
        let addresses = if fixture.is_some() {
            vec![]
        } else {
            resolve_public_addresses(host, limits.connect_timeout).await?
        };
        let fixture_client = match fixture {
            Some(_) => Some(
                media_client(limits, false, None)
                    .map_err(BflDeliveryCause::TransportConfiguration)?,
            ),
            None => None,
        };
        let pinned = if fixture.is_some() {
            Vec::new()
        } else {
            build_pinned(
                limits,
                vec![FalCdnPin {
                    host: host.to_owned(),
                    addresses,
                }],
            )
            .map_err(BflDeliveryCause::TransportConfiguration)?
        };
        Ok(Self {
            limits,
            fixture,
            bfl: true,
            observed_status: std::sync::atomic::AtomicU16::new(0),
            fixture_client,
            pinned,
        })
    }

    /// Whether this fetcher may read `locator`: a BFL delivery URL (or the fixture's own path)
    /// for a BFL fetcher, a strict fal media locator otherwise.
    fn allows_locator(&self, locator: &str) -> bool {
        if self.bfl {
            strict_bfl_delivery(locator)
                || self
                    .fixture
                    .is_some_and(|address| is_fixture_locator(address, locator))
        } else {
            strict_fal_locator(locator)
        }
    }

    /// The pinned pin and client for `host`, when the host was pinned for this owner.
    fn pinned_transport(&self, host: &str) -> Option<&PinnedTransport> {
        self.pinned
            .iter()
            .find(|transport| transport.pin.host == host)
    }

    pub(crate) async fn fetch(
        &self,
        locator: &str,
        mime: &str,
        remaining_bytes: usize,
        deadline: Duration,
        cancellation: impl Future<Output = ()>,
    ) -> Result<Vec<u8>, NativeHttpCause> {
        self.fetch_checked(locator, mime, remaining_bytes, deadline, cancellation)
            .await
            .map(|(bytes, _)| bytes)
    }

    pub(crate) async fn fetch_bfl_image(
        &self,
        locator: &str,
        remaining_bytes: usize,
        deadline: Duration,
    ) -> Result<(Vec<u8>, String), NativeHttpCause> {
        if !self.bfl {
            return Err(NativeHttpCause::InvalidEndpoint);
        }
        self.fetch_checked(
            locator,
            BFL_IMAGE_TYPES,
            remaining_bytes,
            deadline,
            std::future::pending(),
        )
        .await
    }

    /// One download and its `PROVIDER exchange operation=download` audit line. The line names
    /// the locator's host only when the locator is allowed, never its path or query.
    async fn fetch_checked(
        &self,
        locator: &str,
        mime: &str,
        remaining_bytes: usize,
        deadline: Duration,
        cancellation: impl Future<Output = ()>,
    ) -> Result<(Vec<u8>, String), NativeHttpCause> {
        let started = std::time::Instant::now();
        let mut trace = ExchangeTrace::default();
        let result = self
            .fetch_traced(
                locator,
                mime,
                remaining_bytes,
                deadline,
                cancellation,
                &mut trace,
            )
            .await;
        let host = if self.allows_locator(locator) {
            exchange_host(locator)
        } else {
            "-".to_owned()
        };
        ExchangeRecord {
            provider: if self.bfl {
                ProviderName::Bfl
            } else {
                ProviderName::Fal
            },
            operation: Some("download"),
            host: &host,
            status: trace.status,
            cause: match &result {
                Ok(_) => "ok",
                Err(cause) => cause.key(),
            },
            sent: trace.sent,
            request_bytes: 0,
            response_bytes: result.as_ref().map_or(0, |(bytes, _)| bytes.len()),
            duration: started.elapsed(),
            request_id: None,
        }
        .record();
        result
    }

    async fn fetch_traced(
        &self,
        locator: &str,
        mime: &str,
        remaining_bytes: usize,
        deadline: Duration,
        cancellation: impl Future<Output = ()>,
        trace: &mut ExchangeTrace,
    ) -> Result<(Vec<u8>, String), NativeHttpCause> {
        if !self.allows_locator(locator)
            || locator.len() > MAXIMUM_LOCATOR_BYTES
            || remaining_bytes == 0
        {
            return Err(NativeHttpCause::InvalidEndpoint);
        }
        let url = reqwest::Url::parse(locator).map_err(|_| NativeHttpCause::InvalidEndpoint)?;
        let host = url.host_str().ok_or(NativeHttpCause::InvalidEndpoint)?;
        let (target, addresses, client) = if let Some(address) = self.fixture {
            // An explicit operator fixture redirects only the transport socket, never the
            // returned public locator.
            let client = self
                .fixture_client
                .as_ref()
                .ok_or(NativeHttpCause::InvalidEndpoint)?;
            (
                format!("http://{address}{}", url.path()),
                vec![address],
                client,
            )
        } else {
            let transport = self
                .pinned_transport(host)
                .ok_or(NativeHttpCause::InvalidEndpoint)?;
            (
                locator.to_owned(),
                pinned_socket_addresses(&transport.pin),
                &transport.client,
            )
        };
        let duration = deadline.min(self.limits.total_timeout);
        if duration.is_zero() {
            return Err(NativeHttpCause::Deadline);
        }
        tokio::select! {
            biased;
            () = cancellation => Err(NativeHttpCause::Cancelled),
            response = tokio::time::timeout(duration,
                self.read(client, &target, &addresses, mime, remaining_bytes, trace)) => {
                response.map_err(|_| NativeHttpCause::Deadline)?
            }
        }
    }

    async fn read(
        &self,
        client: &reqwest::Client,
        target: &str,
        addresses: &[SocketAddr],
        mime: &str,
        remaining_bytes: usize,
        trace: &mut ExchangeTrace,
    ) -> Result<(Vec<u8>, String), NativeHttpCause> {
        // From here the supplier may have the request, whatever happens next.
        trace.sent = SendState::PossiblySent;
        let mut response = client
            .get(target)
            .send()
            .await
            .map_err(classify_transport_error)?;
        trace.sent = SendState::KnownSent;
        trace.status = Some(response.status().as_u16());
        if self.bfl {
            self.observed_status.store(
                response.status().as_u16(),
                std::sync::atomic::Ordering::Relaxed,
            );
        }
        if !response
            .remote_addr()
            .is_some_and(|address| addresses.contains(&address))
        {
            return Err(NativeHttpCause::InvalidEndpoint);
        }
        if response.status().is_redirection() {
            return Err(NativeHttpCause::Redirect);
        }
        if response.status().as_u16() != 200 {
            return Err(NativeHttpCause::Unknown);
        }
        if header_block_bytes(response.headers())
            .is_none_or(|bytes| bytes > self.limits.maximum_header_bytes)
        {
            return Err(NativeHttpCause::HeadersTooLarge);
        }
        if response
            .headers()
            .contains_key(reqwest::header::CONTENT_ENCODING)
        {
            return Err(NativeHttpCause::ContentType);
        }
        let mut types = response
            .headers()
            .get_all(reqwest::header::CONTENT_TYPE)
            .iter();
        let observed = types
            .next()
            .and_then(|v| v.to_str().ok())
            .ok_or(NativeHttpCause::ContentType)?;
        let observed = observed
            .split(';')
            .next()
            .map(str::trim)
            .unwrap_or("")
            .to_owned();
        let allowed = observed == mime
            || (self.bfl
                && mime == BFL_IMAGE_TYPES
                && matches!(observed.as_str(), "image/png" | "image/jpeg"));
        if types.next().is_some() || !allowed {
            return Err(NativeHttpCause::ContentType);
        }
        let mut lengths = response
            .headers()
            .get_all(reqwest::header::CONTENT_LENGTH)
            .iter();
        let declared = match lengths.next() {
            Some(value) => Some(
                value
                    .to_str()
                    .ok()
                    .and_then(|s| s.parse::<u64>().ok())
                    .ok_or(NativeHttpCause::Unknown)?,
            ),
            None => None,
        };
        if lengths.next().is_some() {
            return Err(NativeHttpCause::Unknown);
        }
        let maximum = remaining_bytes.min(self.limits.maximum_response_bytes);
        if declared.is_some_and(|length| length == 0 || length > maximum as u64) {
            return Err(NativeHttpCause::ResponseTooLarge);
        }
        let body = read_bounded_body(&mut response, maximum, self.limits.maximum_chunks).await?;
        if body.is_empty() || declared.is_some_and(|length| length != body.len() as u64) {
            return Err(NativeHttpCause::Unknown);
        }
        Ok((body, observed))
    }
}

/// Why a BFL delivery failed. Closed and payload-free apart from the transport's own cause.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BflDeliveryCause {
    /// The fixture socket is not a loopback address.
    FixtureInvalid,
    /// The locator is not an allowlisted delivery URL.
    UrlContract,
    UrlParse,
    MissingHost,
    /// The transport refused its own limits or configuration; the transport cause says which.
    TransportConfiguration(NativeHttpCause),
    DnsDeadline,
    DnsFailure,
    /// The delivery host resolved to more addresses than one pin holds.
    DnsAnswerLimit,
    /// The delivery host resolved to an address that is not public.
    DnsTargetRefused,
    /// The requested delivery size is zero or above the transport ceiling.
    RequestBounds,
    Deadline,
    Cancelled,
    RedirectRefused,
    ContentType,
    ResponseTooLarge,
    HeadersTooLarge,
    TooManyChunks,
    RequestTooLarge,
    /// The exchange failed, including any status other than 200.
    FetchFailed,
}

impl BflDeliveryCause {
    /// The audit spelling of this cause.
    pub const fn key(self) -> &'static str {
        match self {
            Self::FixtureInvalid => "fixture_invalid",
            Self::UrlContract => "url_contract",
            Self::UrlParse => "url_parse",
            Self::MissingHost => "missing_host",
            Self::TransportConfiguration(_) => "transport_configuration",
            Self::DnsDeadline => "dns_deadline",
            Self::DnsFailure => "dns_failure",
            Self::DnsAnswerLimit => "dns_answer_limit",
            Self::DnsTargetRefused => "dns_target_refused",
            Self::RequestBounds => "request_bounds",
            Self::Deadline => "deadline",
            Self::Cancelled => "cancelled",
            Self::RedirectRefused => "redirect_refused",
            Self::ContentType => "content_type",
            Self::ResponseTooLarge => "response_too_large",
            Self::HeadersTooLarge => "headers_too_large",
            Self::TooManyChunks => "too_many_chunks",
            Self::RequestTooLarge => "request_too_large",
            Self::FetchFailed => "fetch_failed",
        }
    }

    /// The transport class this failure falls in, for callers that branch on transport causes.
    pub const fn transport(self) -> NativeHttpCause {
        match self {
            Self::FixtureInvalid
            | Self::UrlContract
            | Self::UrlParse
            | Self::MissingHost
            | Self::DnsAnswerLimit
            | Self::DnsTargetRefused => NativeHttpCause::InvalidEndpoint,
            Self::TransportConfiguration(cause) => cause,
            Self::DnsDeadline | Self::Deadline => NativeHttpCause::Deadline,
            Self::DnsFailure | Self::FetchFailed => NativeHttpCause::Unknown,
            Self::RequestBounds => NativeHttpCause::InvalidLimits,
            Self::Cancelled => NativeHttpCause::Cancelled,
            Self::RedirectRefused => NativeHttpCause::Redirect,
            Self::ContentType => NativeHttpCause::ContentType,
            Self::ResponseTooLarge => NativeHttpCause::ResponseTooLarge,
            Self::HeadersTooLarge => NativeHttpCause::HeadersTooLarge,
            Self::TooManyChunks => NativeHttpCause::TooManyChunks,
            Self::RequestTooLarge => NativeHttpCause::RequestTooLarge,
        }
    }

    /// The cause of a failed delivery fetch, from the transport's cause.
    const fn of_fetch(cause: NativeHttpCause) -> Self {
        match cause {
            NativeHttpCause::Deadline => Self::Deadline,
            NativeHttpCause::Cancelled => Self::Cancelled,
            NativeHttpCause::InvalidEndpoint => Self::UrlContract,
            NativeHttpCause::Redirect => Self::RedirectRefused,
            NativeHttpCause::ContentType => Self::ContentType,
            NativeHttpCause::ResponseTooLarge => Self::ResponseTooLarge,
            NativeHttpCause::HeadersTooLarge => Self::HeadersTooLarge,
            NativeHttpCause::TooManyChunks => Self::TooManyChunks,
            NativeHttpCause::RequestTooLarge => Self::RequestTooLarge,
            NativeHttpCause::InvalidLimits
            | NativeHttpCause::InvalidConfiguration
            | NativeHttpCause::InvalidCredential
            | NativeHttpCause::ProviderDisabled => Self::TransportConfiguration(cause),
            NativeHttpCause::Unknown => Self::FetchFailed,
        }
    }
}

impl std::fmt::Display for BflDeliveryCause {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.key())
    }
}

/// Credential-free delivery failure: its closed cause and the observed supplier status.
/// Carries no locator, credential, or body bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BflDeliveryError {
    pub cause: BflDeliveryCause,
    pub http_status: Option<u16>,
}

impl std::fmt::Display for BflDeliveryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "the image delivery failed ({})", self.cause)
    }
}

impl std::error::Error for BflDeliveryError {}

/// The delivery ran out of its one total deadline.
const DELIVERY_DEADLINE: BflDeliveryError = BflDeliveryError {
    cause: BflDeliveryCause::Deadline,
    http_status: None,
};

/// Credential-free BFL delivery over one total `remaining` deadline.
///
/// Builds dedicated credential-free delivery limits from the caller transport
/// with `maximum_response_bytes` set to validated `max_bytes`; the caller
/// transport keeps its submit/poll JSON limits unchanged.
pub async fn fetch_bfl_delivery(
    http: &crate::native_http::NativeHttp,
    locator: &str,
    max_bytes: usize,
    remaining: Duration,
    fixture: Option<SocketAddr>,
) -> Result<(Vec<u8>, String, Option<u16>), BflDeliveryError> {
    if remaining.is_zero() {
        return Err(DELIVERY_DEADLINE);
    }
    if max_bytes == 0 || max_bytes > MAXIMUM_BODY_BYTES {
        return Err(BflDeliveryError {
            cause: BflDeliveryCause::RequestBounds,
            http_status: None,
        });
    }
    let delivery_limits = NativeHttpLimits {
        maximum_response_bytes: max_bytes,
        ..http.limits()
    };
    let started = std::time::Instant::now();
    let constructed = tokio::time::timeout(
        remaining,
        NativeMediaFetch::bfl(delivery_limits, locator, fixture),
    )
    .await
    .map_err(|_| DELIVERY_DEADLINE)?
    .map_err(|cause| BflDeliveryError {
        cause,
        http_status: None,
    })?;
    let left = remaining
        .checked_sub(started.elapsed())
        .filter(|left| !left.is_zero())
        .ok_or(DELIVERY_DEADLINE)?;
    let (bytes, mime) = constructed
        .fetch_bfl_image(locator, max_bytes, left)
        .await
        .map_err(|cause| BflDeliveryError {
            cause: BflDeliveryCause::of_fetch(cause),
            http_status: constructed.observed_status(),
        })?;
    let observed = constructed.observed_status();
    Ok((bytes, mime, observed))
}

fn strict_bfl_delivery(locator: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(locator) else {
        return false;
    };
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && url.fragment().is_none()
        && !url.path().is_empty()
        && url.host_str().is_some_and(|host| {
            // Official outpainting example also uses the exact nonregional delivery host:
            // https://docs.bfl.ml/flux_tools/flux_outpainting
            host == "delivery.bfl.ai"
                || (host.starts_with("delivery.")
                    && host.ends_with(".bfl.ai")
                    && host.len() > "delivery..bfl.ai".len())
        })
        && !locator.contains(['\\', '\0'])
        && locator.len() <= MAXIMUM_LOCATOR_BYTES
}

/// Whether `locator` is a plain path on the loopback fixture `address`: no credentials,
/// fragment, backslash or NUL, and no longer than any other locator.
fn is_fixture_locator(address: SocketAddr, locator: &str) -> bool {
    locator.len() <= MAXIMUM_LOCATOR_BYTES
        && locator.starts_with(&format!("http://{address}/"))
        && !locator.contains(['\\', '\0', '@', '#'])
}

/// Resolves `host` once within `timeout` and keeps its distinct addresses, refusing more than
/// [`MAXIMUM_PINNED`] of them or any that is not public.
async fn resolve_public_addresses(
    host: &str,
    timeout: Duration,
) -> Result<Vec<IpAddr>, BflDeliveryCause> {
    let resolved = tokio::time::timeout(timeout, tokio::net::lookup_host((host, 443)))
        .await
        .map_err(|_| BflDeliveryCause::DnsDeadline)?
        .map_err(|_| BflDeliveryCause::DnsFailure)?;
    let mut addresses = Vec::new();
    for socket in resolved {
        if !addresses.contains(&socket.ip()) {
            addresses.push(socket.ip());
        }
        if addresses.len() > MAXIMUM_PINNED {
            return Err(BflDeliveryCause::DnsAnswerLimit);
        }
    }
    inspect_dns_answers(&addresses).map_err(|_| BflDeliveryCause::DnsTargetRefused)?;
    Ok(addresses)
}

#[cfg(test)]
mod native_media_fetch_tests;
