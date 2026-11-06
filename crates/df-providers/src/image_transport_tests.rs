//! Focused shared image-transport checks: BFL delivery allowlist and deadline.
//!
//! Loopback fixtures only; no live calls. Delivery stays credential-free and
//! shares one total deadline across construction and fetch. Delivery uses
//! dedicated response limits from validated `max_bytes`; the caller transport
//! keeps its submit/poll JSON limits unchanged.

use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener},
    time::Duration,
};

use crate::native_media_fetch::{BflDeliveryCause, BflDeliveryError};
use crate::{NativeHttp, NativeHttpCause, ProviderLimits, fetch_bfl_delivery};

fn limits() -> crate::NativeHttpLimits {
    ProviderLimits::new(Duration::from_secs(5), 1024, 16_384).build()
}

fn json_limits() -> crate::NativeHttpLimits {
    ProviderLimits::new(Duration::from_secs(5), 1024, 64 * 1024).build()
}

fn transport(fixture: bool) -> NativeHttp {
    NativeHttp::new(limits(), fixture).expect("transport")
}

fn transport_with(bounds: crate::NativeHttpLimits, fixture: bool) -> NativeHttp {
    NativeHttp::new(bounds, fixture).expect("transport")
}

fn read_request(stream: &mut std::net::TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("timeout");
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let count = stream.read(&mut chunk).expect("read");
        assert_ne!(count, 0);
        bytes.extend_from_slice(&chunk[..count]);
        if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
            return String::from_utf8(bytes).expect("request text");
        }
    }
}

fn reply(stream: &mut std::net::TcpStream, mime: &str, body: &[u8]) {
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .expect("status");
    stream.write_all(body).expect("body");
}

fn reply_status(stream: &mut std::net::TcpStream, status: u16, mime: &str, body: &[u8]) {
    write!(
        stream,
        "HTTP/1.1 {status} Fixture\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .expect("status");
    stream.write_all(body).expect("body");
}

fn reply_chunked(stream: &mut std::net::TcpStream, mime: &str, body: &[u8]) {
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nConnection: close\r\n\r\n",
    )
    .expect("status");
    stream.write_all(body).expect("body");
}

fn reply_redirect(stream: &mut std::net::TcpStream, target: &str) {
    write!(
        stream,
        "HTTP/1.1 302 Found\r\nLocation: {target}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
    )
    .expect("status");
}

#[tokio::test]
async fn rejected_locator_never_reaches_network() {
    let http = transport(false);
    let error = fetch_bfl_delivery(
        &http,
        "https://untrusted.example/signed-path?secret=value",
        1024,
        Duration::from_secs(1),
        None,
    )
    .await
    .expect_err("untrusted host is refused");
    assert_eq!(error.cause, BflDeliveryCause::UrlContract);
    assert_eq!(error.cause.transport(), NativeHttpCause::InvalidEndpoint);
    assert_eq!(error.http_status, None);
}

#[tokio::test]
async fn rejected_fixture_address_is_invalid_before_fetch() {
    let http = transport(true);
    let outside: SocketAddr = "93.184.216.34:80".parse().expect("address");
    let locator = format!("http://{outside}/image.png");
    let error = fetch_bfl_delivery(&http, &locator, 1024, Duration::from_secs(1), Some(outside))
        .await
        .expect_err("non-loopback fixture is refused");
    assert_eq!(error.cause, BflDeliveryCause::FixtureInvalid);
    assert_eq!(error.cause.transport(), NativeHttpCause::InvalidEndpoint);
    assert_eq!(error.http_status, None);
}

#[tokio::test]
async fn zero_remaining_deadline_covers_the_whole_operation() {
    let http = transport(true);
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let locator = format!("http://{address}/image.png");
    let error = fetch_bfl_delivery(&http, &locator, 1024, Duration::ZERO, Some(address))
        .await
        .expect_err("zero deadline is refused");
    assert_eq!(
        error,
        BflDeliveryError {
            cause: BflDeliveryCause::Deadline,
            http_status: None,
        }
    );
}

#[tokio::test]
async fn slow_fixture_hits_the_single_total_deadline() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let _ = read_request(&mut stream);
        std::thread::sleep(Duration::from_millis(400));
        reply(&mut stream, "image/png", b"late-bytes");
    });
    let http = transport(true);
    let locator = format!("http://{address}/image.png");
    let error = fetch_bfl_delivery(
        &http,
        &locator,
        16_384,
        Duration::from_millis(100),
        Some(address),
    )
    .await
    .expect_err("slow fixture exceeds the total deadline");
    assert_eq!(error.cause, BflDeliveryCause::Deadline);
    server.join().expect("server");
}

#[tokio::test]
async fn pinned_fixture_delivery_returns_bytes_without_a_credential() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let expected: &[u8] = b"fixture-image-bytes";
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let request = read_request(&mut stream).to_ascii_lowercase();
        assert!(!request.contains("fixture-key"));
        assert!(!request.contains("x-key:"));
        assert!(!request.contains("authorization:"));
        reply(&mut stream, "image/png", expected);
    });
    let http = transport(true);
    let locator = format!("http://{address}/image.png");
    let (bytes, mime, status) = fetch_bfl_delivery(
        &http,
        &locator,
        16_384,
        Duration::from_secs(5),
        Some(address),
    )
    .await
    .expect("fixture delivery");
    assert_eq!(bytes.as_slice(), expected);
    assert_eq!(mime, "image/png");
    assert_eq!(status, Some(200));
    server.join().expect("server");
}

#[tokio::test]
async fn delivery_uses_dedicated_response_limit_not_caller_json_limit() {
    let http = transport_with(json_limits(), true);
    assert_eq!(http.limits().maximum_response_bytes, 64 * 1024);
    let body = vec![0x50u8; 128 * 1024];
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let _ = read_request(&mut stream);
        reply(&mut stream, "image/png", &body);
    });
    let locator = format!("http://{address}/image.png");
    let (bytes, mime, status) = fetch_bfl_delivery(
        &http,
        &locator,
        6 * 1024 * 1024,
        Duration::from_secs(5),
        Some(address),
    )
    .await
    .expect("128 KiB delivery succeeds with dedicated 6 MiB limit");
    assert_eq!(bytes.len(), 128 * 1024);
    assert_eq!(mime, "image/png");
    assert_eq!(status, Some(200));
    assert_eq!(http.limits().maximum_response_bytes, 64 * 1024);
    server.join().expect("server");
}

#[tokio::test]
async fn small_delivery_cap_refuses_larger_sized_body() {
    let http = transport(true);
    let body = vec![0x51u8; 2048];
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let _ = read_request(&mut stream);
        reply(&mut stream, "image/png", &body);
    });
    let locator = format!("http://{address}/image.png");
    let error = fetch_bfl_delivery(&http, &locator, 1024, Duration::from_secs(5), Some(address))
        .await
        .expect_err("2 KiB body exceeds 1 KiB delivery cap");
    assert_eq!(error.cause, BflDeliveryCause::ResponseTooLarge);
    assert_eq!(error.cause.transport(), NativeHttpCause::ResponseTooLarge);
    assert_eq!(error.http_status, Some(200));
    server.join().expect("server");
}

#[tokio::test]
async fn small_delivery_cap_refuses_larger_chunked_body() {
    let http = transport(true);
    let body = vec![0x52u8; 2048];
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let _ = read_request(&mut stream);
        reply_chunked(&mut stream, "image/png", &body);
    });
    let locator = format!("http://{address}/image.png");
    let error = fetch_bfl_delivery(&http, &locator, 1024, Duration::from_secs(5), Some(address))
        .await
        .expect_err("chunked 2 KiB body exceeds 1 KiB delivery cap");
    assert_eq!(error.cause, BflDeliveryCause::ResponseTooLarge);
    assert_eq!(error.cause.transport(), NativeHttpCause::ResponseTooLarge);
    assert_eq!(error.http_status, Some(200));
    server.join().expect("server");
}

#[tokio::test]
async fn delivery_403_retains_observed_status_without_payload() {
    let http = transport(true);
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let _ = read_request(&mut stream);
        reply_status(&mut stream, 403, "image/png", b"denied");
    });
    let locator = format!("http://{address}/image.png");
    let error = fetch_bfl_delivery(
        &http,
        &locator,
        16_384,
        Duration::from_secs(5),
        Some(address),
    )
    .await
    .expect_err("403 delivery is refused with its status");
    assert_eq!(error.cause, BflDeliveryCause::FetchFailed);
    assert_eq!(error.cause.transport(), NativeHttpCause::Unknown);
    assert_eq!(error.http_status, Some(403));
    server.join().expect("server");
}

#[tokio::test]
async fn redirect_fixture_is_refused_without_following() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let _ = read_request(&mut stream);
        reply_redirect(&mut stream, "http://127.0.0.1/other.png");
    });
    let http = transport(true);
    let locator = format!("http://{address}/image.png");
    let error = fetch_bfl_delivery(
        &http,
        &locator,
        16_384,
        Duration::from_secs(5),
        Some(address),
    )
    .await
    .expect_err("redirect is refused");
    assert_eq!(error.cause, BflDeliveryCause::RedirectRefused);
    assert_eq!(error.cause.transport(), NativeHttpCause::Redirect);
    server.join().expect("server");
}

/// The delivery bound used to stop at 16 MiB while the transport allows far more, so a caller
/// with a larger delivery bound was refused with `request_bounds` before any fetch.
#[tokio::test]
async fn a_delivery_bound_above_sixteen_mebibytes_is_accepted() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let _ = read_request(&mut stream);
        reply(&mut stream, "image/png", b"large-bound-bytes");
    });
    let http = transport(true);
    let locator = format!("http://{address}/image.png");
    let (bytes, _, _) = fetch_bfl_delivery(
        &http,
        &locator,
        32 * 1024 * 1024,
        Duration::from_secs(5),
        Some(address),
    )
    .await
    .expect("a 32 MiB delivery bound is within the transport ceiling");
    assert_eq!(bytes, b"large-bound-bytes");
    server.join().expect("server");
}

#[tokio::test]
async fn unsupported_delivery_bounds_are_refused_before_dns() {
    let http = transport(false);
    let locator = "https://delivery.bfl.ai/image.png?signature=synthetic";
    for max_bytes in [0, crate::MAXIMUM_BODY_BYTES + 1] {
        let error = fetch_bfl_delivery(&http, locator, max_bytes, Duration::from_secs(1), None)
            .await
            .expect_err("unsupported delivery bound is refused");
        assert_eq!(error.cause, BflDeliveryCause::RequestBounds);
        assert_eq!(error.cause.transport(), NativeHttpCause::InvalidLimits);
        assert_eq!(error.http_status, None);
    }
}

#[tokio::test]
async fn retry_after_evidence_preserves_absent_invalid_duplicate_seconds_and_date() {
    use crate::{HttpMethod, HttpRequest, NativeRetryAfterValue};
    use std::time::{Instant, SystemTime, UNIX_EPOCH};
    let date = UNIX_EPOCH + Duration::from_secs(1_445_412_480);
    for (headers, raw, expected) in [
        ("", vec![], NativeRetryAfterValue::Absent),
        (
            "Retry-After: 12\r\n",
            vec![b"12".to_vec()],
            NativeRetryAfterValue::DelaySeconds(12),
        ),
        (
            "Retry-After: Wed, 21 Oct 2015 07:28:00 GMT\r\n",
            vec![b"Wed, 21 Oct 2015 07:28:00 GMT".to_vec()],
            NativeRetryAfterValue::HttpDate(date),
        ),
        (
            "Retry-After: nonsense\r\n",
            vec![b"nonsense".to_vec()],
            NativeRetryAfterValue::Invalid,
        ),
        (
            "Retry-After: -1\r\n",
            vec![b"-1".to_vec()],
            NativeRetryAfterValue::Invalid,
        ),
        (
            "Retry-After: 1.5\r\n",
            vec![b"1.5".to_vec()],
            NativeRetryAfterValue::Invalid,
        ),
        (
            "Retry-After: 1\r\nRetry-After: 1\r\n",
            vec![b"1".to_vec(), b"1".to_vec()],
            NativeRetryAfterValue::Duplicate,
        ),
        (
            "Retry-After: 18446744073709551616\r\n",
            vec![b"18446744073709551616".to_vec()],
            NativeRetryAfterValue::Invalid,
        ),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        listener.set_nonblocking(true).expect("bounded accept");
        let address = listener.local_addr().expect("address");
        let server = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        std::thread::sleep(Duration::from_millis(1))
                    }
                    Err(error) => panic!("bounded fixture accept: {error}"),
                }
            };
            stream.set_nonblocking(false).expect("blocking stream");
            stream
                .set_write_timeout(Some(Duration::from_secs(2)))
                .expect("write deadline");
            let _ = read_request(&mut stream);
            write!(stream, "HTTP/1.1 429 Fixture\r\nContent-Type: application/json\r\n{headers}Content-Length: 2\r\nConnection: close\r\n\r\n{{}}")
                .expect("response");
        });
        let response = transport(true)
            .execute(
                HttpRequest::bfl_native(
                    HttpMethod::Get,
                    &format!("http://{address}/fixture"),
                    "fixture-key",
                    Vec::new(),
                ),
                Duration::from_secs(5),
                std::future::pending(),
            )
            .await
            .unwrap_or_else(|failure| panic!("transport refused: {:?}", failure.cause));
        assert_eq!(response.status, 429);
        assert_eq!(response.body, b"{}");
        assert_eq!(response.metadata.retry_after.raw_values(), raw);
        assert_eq!(response.metadata.retry_after.value(), expected);
        let replay = crate::NativeRetryAfter::from_raw_values(raw);
        assert_eq!(replay, response.metadata.retry_after);
        assert_eq!(
            replay.minimum_delay(SystemTime::now()).is_some(),
            matches!(
                expected,
                NativeRetryAfterValue::Absent
                    | NativeRetryAfterValue::DelaySeconds(_)
                    | NativeRetryAfterValue::HttpDate(_)
            )
        );
        server.join().expect("fixture");
    }
}

#[test]
fn retry_after_delay_is_checked_and_does_not_reveal_header_prose() {
    use crate::NativeRetryAfter;
    use std::time::UNIX_EPOCH;
    let seconds = NativeRetryAfter::from_raw_values(vec![b"12".to_vec()]);
    assert_eq!(
        seconds.minimum_delay(UNIX_EPOCH),
        Some(Duration::from_secs(12))
    );
    let date = NativeRetryAfter::from_raw_values(vec![b"Wed, 21 Oct 2015 07:28:00 GMT".to_vec()]);
    assert_eq!(
        date.minimum_delay(UNIX_EPOCH),
        Some(Duration::from_secs(1_445_412_480))
    );
    assert_eq!(
        date.minimum_delay(UNIX_EPOCH + Duration::from_secs(1_445_412_481)),
        Some(Duration::ZERO)
    );
    assert_eq!(
        NativeRetryAfter::from_raw_values(vec![u64::MAX.to_string().into_bytes()])
            .minimum_delay(UNIX_EPOCH),
        None
    );
    let invalid = NativeRetryAfter::from_raw_values(vec![b"private supplier prose".to_vec()]);
    assert!(!format!("{invalid:?}").contains("private supplier prose"));
    assert_eq!(invalid.minimum_delay(UNIX_EPOCH), None);
}
