//! Credential-free bounded native media acquisition from explicit socket pins.

use std::{
    future::Future,
    net::{IpAddr, SocketAddr},
    time::Duration,
};

use crate::native_http::{
    MAXIMUM_BODY_BYTES, NativeHttp, classify_transport_error, hardened_client_builder,
    header_block_bytes, is_loopback_socket, read_bounded_body,
};
use crate::output_fetch::{MAXIMUM_LOCATOR_BYTES, strict_fal_locator};
use crate::{NativeHttpLimits, NativeHttpRefusal, inspect_dns_answers};

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

impl FalCdnPin {
    /// Public, finite DNS observations are caller-supplied. No resolver fallback is enabled.
    pub fn new(host: &str, addresses: &[IpAddr]) -> Result<Self, NativeHttpRefusal> {
        if !strict_fal_locator(&format!("https://{host}/pin")) {
            return Err(NativeHttpRefusal::InvalidEndpoint);
        }
        let inspected =
            inspect_dns_answers(addresses).map_err(|_| NativeHttpRefusal::InvalidEndpoint)?;
        if addresses.iter().enumerate().any(|(i, value)| {
            addresses
                .get(..i)
                .is_some_and(|earlier| earlier.contains(value))
        }) {
            return Err(NativeHttpRefusal::InvalidEndpoint);
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
) -> Result<Vec<PinnedTransport>, NativeHttpRefusal> {
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
) -> Result<reqwest::Client, NativeHttpRefusal> {
    let mut builder = hardened_client_builder(&limits, https_only)
        .pool_max_idle_per_host(MEDIA_POOL_IDLE_PER_HOST)
        .pool_idle_timeout(MEDIA_POOL_IDLE_TIMEOUT);
    if let Some((host, addresses)) = resolve {
        builder = builder.resolve_to_addrs(host, addresses);
    }
    builder.build().map_err(|_| NativeHttpRefusal::Unknown)
}

impl NativeMediaFetch {
    pub(crate) fn new(
        limits: NativeHttpLimits,
        pins: Vec<FalCdnPin>,
        fixture: Option<SocketAddr>,
    ) -> Result<Self, NativeHttpRefusal> {
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
            return Err(NativeHttpRefusal::InvalidLimits);
        }
        // Reuse the validated finite native transport bounds without any network operation.
        NativeHttp::new(limits, fixture.is_some())?;
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
    ) -> Result<Self, (NativeHttpRefusal, &'static str)> {
        if let Some(fixture_address) = fixture {
            if !is_loopback_socket(fixture_address) {
                return Err((NativeHttpRefusal::InvalidEndpoint, "fixture_invalid"));
            }
            if !(strict_bfl_delivery(locator) || is_fixture_locator(fixture_address, locator)) {
                return Err((NativeHttpRefusal::InvalidEndpoint, "url_contract"));
            }
        } else if !strict_bfl_delivery(locator) {
            return Err((NativeHttpRefusal::InvalidEndpoint, "url_contract"));
        }
        let url = reqwest::Url::parse(locator)
            .map_err(|_| (NativeHttpRefusal::InvalidEndpoint, "url_parse"))?;
        let host = url
            .host_str()
            .ok_or((NativeHttpRefusal::InvalidEndpoint, "missing_host"))?;
        // Validate transport bounds before DNS so unsupported limits never cause network I/O.
        NativeHttp::new(limits, fixture.is_some())
            .map_err(|error| (error, "transport_configuration"))?;
        let addresses = if fixture.is_some() {
            vec![]
        } else {
            resolve_public_addresses(host, limits.connect_timeout).await?
        };
        let fixture_client = match fixture {
            Some(_) => Some(
                media_client(limits, false, None)
                    .map_err(|error| (error, "transport_configuration"))?,
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
            .map_err(|error| (error, "transport_configuration"))?
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
    ) -> Result<Vec<u8>, NativeHttpRefusal> {
        self.fetch_checked(locator, mime, remaining_bytes, deadline, cancellation)
            .await
            .map(|(bytes, _)| bytes)
    }

    pub(crate) async fn fetch_bfl_image(
        &self,
        locator: &str,
        remaining_bytes: usize,
        deadline: Duration,
    ) -> Result<(Vec<u8>, String), NativeHttpRefusal> {
        if !self.bfl {
            return Err(NativeHttpRefusal::InvalidEndpoint);
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

    async fn fetch_checked(
        &self,
        locator: &str,
        mime: &str,
        remaining_bytes: usize,
        deadline: Duration,
        cancellation: impl Future<Output = ()>,
    ) -> Result<(Vec<u8>, String), NativeHttpRefusal> {
        if !self.allows_locator(locator)
            || locator.len() > MAXIMUM_LOCATOR_BYTES
            || remaining_bytes == 0
        {
            return Err(NativeHttpRefusal::InvalidEndpoint);
        }
        let url = reqwest::Url::parse(locator).map_err(|_| NativeHttpRefusal::InvalidEndpoint)?;
        let host = url.host_str().ok_or(NativeHttpRefusal::InvalidEndpoint)?;
        let (target, addresses, client) = if let Some(address) = self.fixture {
            // An explicit operator fixture redirects only the transport socket, never the
            // returned public locator.
            let client = self
                .fixture_client
                .as_ref()
                .ok_or(NativeHttpRefusal::InvalidEndpoint)?;
            (
                format!("http://{address}{}", url.path()),
                vec![address],
                client,
            )
        } else {
            let transport = self
                .pinned_transport(host)
                .ok_or(NativeHttpRefusal::InvalidEndpoint)?;
            (
                locator.to_owned(),
                pinned_socket_addresses(&transport.pin),
                &transport.client,
            )
        };
        let duration = deadline.min(self.limits.total_timeout);
        if duration.is_zero() {
            return Err(NativeHttpRefusal::Deadline);
        }
        tokio::select! {
            biased;
            () = cancellation => Err(NativeHttpRefusal::Cancelled),
            response = tokio::time::timeout(duration,
                self.read(client, &target, &addresses, mime, remaining_bytes)) => {
                response.map_err(|_| NativeHttpRefusal::Deadline)?
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
    ) -> Result<(Vec<u8>, String), NativeHttpRefusal> {
        let mut response = client
            .get(target)
            .send()
            .await
            .map_err(classify_transport_error)?;
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
            return Err(NativeHttpRefusal::InvalidEndpoint);
        }
        if response.status().is_redirection() {
            return Err(NativeHttpRefusal::Redirect);
        }
        if response.status().as_u16() != 200 {
            return Err(NativeHttpRefusal::Unknown);
        }
        if header_block_bytes(response.headers())
            .is_none_or(|bytes| bytes > self.limits.maximum_header_bytes)
        {
            return Err(NativeHttpRefusal::HeadersTooLarge);
        }
        if response
            .headers()
            .contains_key(reqwest::header::CONTENT_ENCODING)
        {
            return Err(NativeHttpRefusal::ContentType);
        }
        let mut types = response
            .headers()
            .get_all(reqwest::header::CONTENT_TYPE)
            .iter();
        let observed = types
            .next()
            .and_then(|v| v.to_str().ok())
            .ok_or(NativeHttpRefusal::ContentType)?;
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
            return Err(NativeHttpRefusal::ContentType);
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
                    .ok_or(NativeHttpRefusal::Unknown)?,
            ),
            None => None,
        };
        if lengths.next().is_some() {
            return Err(NativeHttpRefusal::Unknown);
        }
        let maximum = remaining_bytes.min(self.limits.maximum_response_bytes);
        if declared.is_some_and(|length| length == 0 || length > maximum as u64) {
            return Err(NativeHttpRefusal::ResponseTooLarge);
        }
        let body = read_bounded_body(&mut response, maximum, self.limits.maximum_chunks).await?;
        if body.is_empty() || declared.is_some_and(|length| length != body.len() as u64) {
            return Err(NativeHttpRefusal::Unknown);
        }
        Ok((body, observed))
    }
}

/// Credential-free delivery failure; closed refusal, static reason, observed status.
/// Carries no locator, credential, or body bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BflDeliveryFailure {
    pub refusal: NativeHttpRefusal,
    pub reason: &'static str,
    pub http_status: Option<u16>,
}

/// The delivery ran out of its one total deadline.
const DELIVERY_DEADLINE: BflDeliveryFailure = BflDeliveryFailure {
    refusal: NativeHttpRefusal::Deadline,
    reason: "deadline",
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
) -> Result<(Vec<u8>, String, Option<u16>), BflDeliveryFailure> {
    if remaining.is_zero() {
        return Err(DELIVERY_DEADLINE);
    }
    if max_bytes == 0 || max_bytes > MAXIMUM_BODY_BYTES {
        return Err(BflDeliveryFailure {
            refusal: NativeHttpRefusal::InvalidLimits,
            reason: "request_bounds",
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
    .map_err(|(refusal, reason)| BflDeliveryFailure {
        refusal,
        reason,
        http_status: None,
    })?;
    let left = remaining
        .checked_sub(started.elapsed())
        .filter(|left| !left.is_zero())
        .ok_or(DELIVERY_DEADLINE)?;
    let (bytes, mime) = constructed
        .fetch_bfl_image(locator, max_bytes, left)
        .await
        .map_err(|refusal| BflDeliveryFailure {
            refusal,
            reason: fetch_reason(refusal),
            http_status: constructed.observed_status(),
        })?;
    let observed = constructed.observed_status();
    Ok((bytes, mime, observed))
}

/// Closed static reason for a delivery fetch refusal; no payload is carried.
fn fetch_reason(reason: NativeHttpRefusal) -> &'static str {
    match reason {
        NativeHttpRefusal::Deadline => "deadline",
        NativeHttpRefusal::Cancelled => "cancelled",
        NativeHttpRefusal::InvalidEndpoint => "url_contract",
        NativeHttpRefusal::Redirect => "redirect_refused",
        NativeHttpRefusal::ContentType => "content_type",
        NativeHttpRefusal::ResponseTooLarge => "response_too_large",
        NativeHttpRefusal::HeadersTooLarge => "headers_too_large",
        NativeHttpRefusal::TooManyChunks => "too_many_chunks",
        NativeHttpRefusal::RequestTooLarge => "request_too_large",
        NativeHttpRefusal::InvalidLimits
        | NativeHttpRefusal::InvalidConfiguration
        | NativeHttpRefusal::InvalidCredential
        | NativeHttpRefusal::ProviderDisabled => "transport_configuration",
        NativeHttpRefusal::Unknown => "fetch_failed",
    }
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
) -> Result<Vec<IpAddr>, (NativeHttpRefusal, &'static str)> {
    let resolved = tokio::time::timeout(timeout, tokio::net::lookup_host((host, 443)))
        .await
        .map_err(|_| (NativeHttpRefusal::Deadline, "dns_deadline"))?
        .map_err(|_| (NativeHttpRefusal::Unknown, "dns_failure"))?;
    let mut addresses = Vec::new();
    for socket in resolved {
        if !addresses.contains(&socket.ip()) {
            addresses.push(socket.ip());
        }
        if addresses.len() > MAXIMUM_PINNED {
            return Err((NativeHttpRefusal::InvalidEndpoint, "dns_answer_limit"));
        }
    }
    inspect_dns_answers(&addresses)
        .map_err(|_| (NativeHttpRefusal::InvalidEndpoint, "dns_target_refused"))?;
    Ok(addresses)
}

#[cfg(test)]
mod native_media_fetch_tests;
