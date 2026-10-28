//! Focused shared image-transport checks: BFL delivery allowlist and deadline.
//!
//! Loopback fixtures only; no live calls. Delivery stays credential-free and
//! shares one total deadline across construction and fetch.

use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener},
    time::Duration,
};

use crate::{NativeHttp, NativeHttpRefusal, ProviderLimits, fetch_bfl_delivery};

fn limits() -> crate::NativeHttpLimits {
    ProviderLimits::new(Duration::from_secs(5), 1024, 16_384).build()
}

fn transport(fixture: bool) -> NativeHttp {
    NativeHttp::new(limits(), fixture).expect("transport")
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
    assert_eq!(error, (NativeHttpRefusal::InvalidEndpoint, "url_contract"));
}

#[tokio::test]
async fn rejected_fixture_address_is_invalid_before_fetch() {
    let http = transport(true);
    let outside: SocketAddr = "93.184.216.34:80".parse().expect("address");
    let locator = format!("http://{outside}/image.png");
    let error = fetch_bfl_delivery(
        &http,
        &locator,
        1024,
        Duration::from_secs(1),
        Some(outside),
    )
    .await
    .expect_err("non-loopback fixture is refused");
    assert_eq!(
        error,
        (NativeHttpRefusal::InvalidEndpoint, "fixture_invalid")
    );
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
    assert_eq!(error, (NativeHttpRefusal::Deadline, "deadline"));
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
    assert_eq!(error.0, NativeHttpRefusal::Deadline);
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
    let (bytes, mime, status) =
        fetch_bfl_delivery(&http, &locator, 16_384, Duration::from_secs(5), Some(address))
            .await
            .expect("fixture delivery");
    assert_eq!(bytes.as_slice(), expected);
    assert_eq!(mime, "image/png");
    assert_eq!(status, Some(200));
    server.join().expect("server");
}
