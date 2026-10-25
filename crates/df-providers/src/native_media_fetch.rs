//! Credential-free bounded native media acquisition from explicit socket pins.

use std::{
    future::Future,
    net::{IpAddr, SocketAddr},
    time::Duration,
};

use crate::output_fetch::strict_fal_locator;
use crate::{NativeHttpLimits, NativeHttpRefusal, inspect_dns_answers};

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
    let mut builder = reqwest::Client::builder()
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
        if limits.maximum_header_bytes > 16_384
            || pins.len() > 16
            || pins.iter().map(|pin| pin.addresses.len()).sum::<usize>() > 16
            || pins.iter().enumerate().any(|(i, pin)| {
                pins.get(..i)
                    .is_some_and(|earlier| earlier.iter().any(|old| old.host == pin.host))
            })
            || fixture.is_some_and(|address| !address.ip().is_loopback() || address.port() == 0)
            || (fixture.is_none() && pins.is_empty())
        {
            return Err(NativeHttpRefusal::InvalidLimits);
        }
        // Reuse the validated finite native transport bounds without any network operation.
        crate::native_http::NativeHttp::new(limits, fixture.is_some())?;
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
        if !strict_bfl_delivery(locator) {
            return Err((NativeHttpRefusal::InvalidEndpoint, "url_contract"));
        }
        let url = reqwest::Url::parse(locator)
            .map_err(|_| (NativeHttpRefusal::InvalidEndpoint, "url_parse"))?;
        let host = url
            .host_str()
            .ok_or((NativeHttpRefusal::InvalidEndpoint, "missing_host"))?;
        let addresses = if fixture.is_some() {
            vec![]
        } else {
            let resolved =
                tokio::time::timeout(limits.connect_timeout, tokio::net::lookup_host((host, 443)))
                    .await
                    .map_err(|_| (NativeHttpRefusal::Deadline, "dns_deadline"))?
                    .map_err(|_| (NativeHttpRefusal::Unknown, "dns_failure"))?;
            let mut addresses = Vec::new();
            for socket in resolved {
                if !addresses.contains(&socket.ip()) {
                    addresses.push(socket.ip());
                }
                if addresses.len() > 16 {
                    return Err((NativeHttpRefusal::InvalidEndpoint, "dns_answer_limit"));
                }
            }
            inspect_dns_answers(&addresses)
                .map_err(|_| (NativeHttpRefusal::InvalidEndpoint, "dns_target_refused"))?;
            addresses
        };
        crate::native_http::NativeHttp::new(limits, fixture.is_some())
            .map_err(|error| (error, "transport_configuration"))?;
        if fixture.is_some_and(|socket| !socket.ip().is_loopback() || socket.port() == 0) {
            return Err((NativeHttpRefusal::InvalidEndpoint, "fixture_invalid"));
        }
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
            "image/png,image/jpeg",
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
        let allowed = if self.bfl {
            strict_bfl_delivery(locator)
        } else {
            strict_fal_locator(locator)
        };
        if !allowed || locator.len() > 8192 || remaining_bytes == 0 {
            return Err(NativeHttpRefusal::InvalidEndpoint);
        }
        let url = reqwest::Url::parse(locator).map_err(|_| NativeHttpRefusal::InvalidEndpoint)?;
        let host = url.host_str().ok_or(NativeHttpRefusal::InvalidEndpoint)?;
        let (target, addresses, client) = if let Some(address) = self.fixture {
            // Explicit operator fixture redirects only the transport socket, never the returned public locator.
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
        let mut response = client.get(target).send().await.map_err(transport_error)?;
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
        let mut headers = 0usize;
        for (name, value) in response.headers() {
            headers = headers
                .checked_add(name.as_str().len())
                .and_then(|v| v.checked_add(value.as_bytes().len()))
                .ok_or(NativeHttpRefusal::HeadersTooLarge)?;
        }
        if headers > self.limits.maximum_header_bytes {
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
                && mime == "image/png,image/jpeg"
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
        let mut body = Vec::new();
        let mut chunks = 0usize;
        while let Some(chunk) = response.chunk().await.map_err(transport_error)? {
            chunks += 1;
            if chunks > self.limits.maximum_chunks {
                return Err(NativeHttpRefusal::TooManyChunks);
            }
            if body
                .len()
                .checked_add(chunk.len())
                .is_none_or(|length| length > maximum)
            {
                return Err(NativeHttpRefusal::ResponseTooLarge);
            }
            body.try_reserve(chunk.len())
                .map_err(|_| NativeHttpRefusal::ResponseTooLarge)?;
            body.extend_from_slice(&chunk);
        }
        if body.is_empty() || declared.is_some_and(|length| length != body.len() as u64) {
            return Err(NativeHttpRefusal::Unknown);
        }
        Ok((body, observed))
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
        && locator.len() <= 8192
}

#[cfg(test)]
mod bfl_tests {
    use super::*;
    #[tokio::test]
    async fn rejected_delivery_has_closed_admission_reason_without_network() {
        let limits = crate::ProviderLimits::new(Duration::from_secs(1), 1, 1024).build();
        let error = NativeMediaFetch::bfl(
            limits,
            "https://untrusted.example/signed-path?secret=value",
            None,
        )
        .await
        .err()
        .unwrap();
        assert_eq!(error, (NativeHttpRefusal::InvalidEndpoint, "url_contract"));
    }
    #[test]
    fn delivery_allowlist_excludes_private_or_credentialed_urls() {
        assert!(strict_bfl_delivery(
            "https://delivery.bfl.ai/image.png?signature=synthetic"
        ));
        assert!(strict_bfl_delivery(
            "https://delivery.us1.bfl.ai/image.png?signature=synthetic"
        ));
        for locator in [
            "http://delivery.us1.bfl.ai/image.png",
            "https://delivery.bfl.ai.evil.test/image.png",
            "https://delivery.bfl.ai:444/image.png",
            "https://user@delivery.bfl.ai/image.png",
            "https://delivery-bfl.ai/image.png",
            "https://api.bfl.ai/image.png",
            "https://127.0.0.1/image.png",
            "https://delivery.us1.bfl.ai.evil.test/image.png",
            "https://delivery.us1.bfl.ai:444/image.png",
            "https://user@delivery.us1.bfl.ai/image.png",
            "https://delivery.us1.bfl.ai/image.png#fragment",
        ] {
            assert!(!strict_bfl_delivery(locator));
        }
    }
}

#[cfg(test)]
mod client_reuse_tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    /// A keep-alive fixture: answers every request on each accepted socket with `body` and
    /// counts accepted sockets. The accept loop is returned so the test can stop it.
    async fn keep_alive_fixture(
        body: &'static [u8],
    ) -> (SocketAddr, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let accepted = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&accepted);
        let accept_loop = tokio::spawn(async move {
            // Owned connections: aborting the accept loop drops this set and aborts each socket task.
            let mut connections = tokio::task::JoinSet::new();
            while let Ok((mut socket, _)) = listener.accept().await {
                counter.fetch_add(1, Ordering::SeqCst);
                connections.spawn(async move {
                    let mut pending = Vec::new();
                    let mut chunk = [0u8; 1024];
                    loop {
                        while !pending.windows(4).any(|window| window == b"\r\n\r\n") {
                            match socket.read(&mut chunk).await {
                                Ok(0) | Err(_) => return,
                                Ok(read) => pending.extend_from_slice(&chunk[..read]),
                            }
                        }
                        let end = pending.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
                        pending.drain(..end);
                        let head = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\n\r\n",
                            body.len()
                        );
                        if socket.write_all(head.as_bytes()).await.is_err()
                            || socket.write_all(body).await.is_err()
                        {
                            return;
                        }
                    }
                });
            }
        });
        (address, accepted, accept_loop)
    }

    /// Two files of one response share one pooled connection. Before the shared client, each
    /// fetch built its own client and opened its own socket, so this observed two sockets.
    #[tokio::test]
    async fn sequential_media_fetches_reuse_one_connection() {
        let body: &'static [u8] = b"synthetic-media-bytes";
        let (address, accepted, accept_loop) = keep_alive_fixture(body).await;
        let limits = crate::ProviderLimits::new(Duration::from_secs(5), 1024, 1024).build();
        let fetch = NativeMediaFetch::new(limits, Vec::new(), Some(address)).unwrap();
        let locator = "https://fal.media/files/first.png";
        for _ in 0..2 {
            let bytes = fetch
                .fetch(
                    locator,
                    "image/png",
                    1024,
                    Duration::from_secs(5),
                    std::future::pending(),
                )
                .await
                .unwrap();
            assert_eq!(bytes.as_slice(), body);
        }
        accept_loop.abort();
        assert_eq!(accepted.load(Ordering::SeqCst), 1);
    }

    /// Each pinned host finds its own pin and that pin's addresses, and an unpinned host finds
    /// nothing. The client is stored in the same entry as its pin, so the lookup cannot pair a
    /// client built for one host with another host's addresses.
    #[test]
    fn pinned_lookup_pairs_each_host_with_its_own_addresses() {
        let first: IpAddr = "93.184.216.34".parse().unwrap();
        let second: IpAddr = "1.1.1.1".parse().unwrap();
        let pins = vec![
            FalCdnPin::new("a.fal.media", &[first]).unwrap(),
            FalCdnPin::new("b.fal.media", &[second]).unwrap(),
        ];
        let limits = crate::ProviderLimits::new(Duration::from_secs(5), 1024, 1024).build();
        let fetch = NativeMediaFetch::new(limits, pins, None).unwrap();

        let second_transport = fetch.pinned_transport("b.fal.media").unwrap();
        assert_eq!(second_transport.pin.host, "b.fal.media");
        assert_eq!(
            pinned_socket_addresses(&second_transport.pin),
            vec![SocketAddr::new(second, 443)]
        );
        let first_transport = fetch.pinned_transport("a.fal.media").unwrap();
        assert_eq!(first_transport.pin.host, "a.fal.media");
        assert_eq!(
            pinned_socket_addresses(&first_transport.pin),
            vec![SocketAddr::new(first, 443)]
        );
        assert!(fetch.pinned_transport("c.fal.media").is_none());
    }
}

fn transport_error(error: reqwest::Error) -> NativeHttpRefusal {
    if error.is_timeout() {
        NativeHttpRefusal::Deadline
    } else {
        NativeHttpRefusal::Unknown
    }
}
