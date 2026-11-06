use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

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
    assert_eq!(error, BflDeliveryCause::UrlContract);
}

/// Every delivery cause keeps the audit spelling the string reasons had, and maps to the
/// transport cause the old pair carried.
#[test]
fn delivery_cause_keys_and_transport_classes_are_stable() {
    let cases = [
        (
            BflDeliveryCause::FixtureInvalid,
            "fixture_invalid",
            NativeHttpCause::InvalidEndpoint,
        ),
        (
            BflDeliveryCause::UrlContract,
            "url_contract",
            NativeHttpCause::InvalidEndpoint,
        ),
        (
            BflDeliveryCause::UrlParse,
            "url_parse",
            NativeHttpCause::InvalidEndpoint,
        ),
        (
            BflDeliveryCause::MissingHost,
            "missing_host",
            NativeHttpCause::InvalidEndpoint,
        ),
        (
            BflDeliveryCause::TransportConfiguration(NativeHttpCause::Unknown),
            "transport_configuration",
            NativeHttpCause::Unknown,
        ),
        (
            BflDeliveryCause::DnsDeadline,
            "dns_deadline",
            NativeHttpCause::Deadline,
        ),
        (
            BflDeliveryCause::DnsFailure,
            "dns_failure",
            NativeHttpCause::Unknown,
        ),
        (
            BflDeliveryCause::DnsAnswerLimit,
            "dns_answer_limit",
            NativeHttpCause::InvalidEndpoint,
        ),
        (
            BflDeliveryCause::DnsTargetRefused,
            "dns_target_refused",
            NativeHttpCause::InvalidEndpoint,
        ),
        (
            BflDeliveryCause::RequestBounds,
            "request_bounds",
            NativeHttpCause::InvalidLimits,
        ),
        (
            BflDeliveryCause::Deadline,
            "deadline",
            NativeHttpCause::Deadline,
        ),
        (
            BflDeliveryCause::Cancelled,
            "cancelled",
            NativeHttpCause::Cancelled,
        ),
        (
            BflDeliveryCause::RedirectRefused,
            "redirect_refused",
            NativeHttpCause::Redirect,
        ),
        (
            BflDeliveryCause::ContentType,
            "content_type",
            NativeHttpCause::ContentType,
        ),
        (
            BflDeliveryCause::ResponseTooLarge,
            "response_too_large",
            NativeHttpCause::ResponseTooLarge,
        ),
        (
            BflDeliveryCause::HeadersTooLarge,
            "headers_too_large",
            NativeHttpCause::HeadersTooLarge,
        ),
        (
            BflDeliveryCause::TooManyChunks,
            "too_many_chunks",
            NativeHttpCause::TooManyChunks,
        ),
        (
            BflDeliveryCause::RequestTooLarge,
            "request_too_large",
            NativeHttpCause::RequestTooLarge,
        ),
        (
            BflDeliveryCause::FetchFailed,
            "fetch_failed",
            NativeHttpCause::Unknown,
        ),
    ];
    for (cause, key, transport) in cases {
        assert_eq!(cause.key(), key);
        assert_eq!(cause.transport(), transport, "{key}");
    }
}

/// A fetch failure keeps the transport cause it came from, so a caller that branches on the
/// transport class sees the same cause as before the delivery cause was typed.
#[test]
fn fetch_failures_round_trip_their_transport_cause() {
    for cause in [
        NativeHttpCause::InvalidLimits,
        NativeHttpCause::InvalidConfiguration,
        NativeHttpCause::InvalidEndpoint,
        NativeHttpCause::InvalidCredential,
        NativeHttpCause::ProviderDisabled,
        NativeHttpCause::RequestTooLarge,
        NativeHttpCause::ResponseTooLarge,
        NativeHttpCause::HeadersTooLarge,
        NativeHttpCause::TooManyChunks,
        NativeHttpCause::ContentType,
        NativeHttpCause::Redirect,
        NativeHttpCause::Cancelled,
        NativeHttpCause::Deadline,
        NativeHttpCause::Unknown,
    ] {
        assert_eq!(BflDeliveryCause::of_fetch(cause).transport(), cause);
    }
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
                        concat!(
                            "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\n",
                            "Content-Length: {}\r\n\r\n"
                        ),
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
