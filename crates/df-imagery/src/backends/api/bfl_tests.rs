//! Failure-focused loopback tests for the private BFL backend.
//!
//! No live calls. Moderation stays terminal, delivery stays credential-free,
//! and submit/poll/download boundaries are diagnosed separately.

use super::bfl::{BflConfig, BflExecutor};
use super::{ApiError, ImageBackground, ImageEdit, ImageFormat, ImageModeration, ImageQuality};
use super::{ImageEditModel, OperationalBounds};
use std::{
    io::{Read, Write},
    net::TcpListener,
    time::Duration,
};

fn bounds() -> OperationalBounds {
    OperationalBounds::compatibility_defaults()
}

fn portrait<'a>(prompt: &'a str) -> ImageEdit<'a> {
    ImageEdit {
        model: ImageEditModel::Gpt25Flare,
        prompt,
        references: &[],
        width: 1024,
        height: 1536,
        quality: ImageQuality::Medium,
        background: ImageBackground::Opaque,
        output: ImageFormat::Jpeg,
        moderation: ImageModeration::Auto,
    }
}

fn minimal_jpeg(width: u16, height: u16) -> Vec<u8> {
    vec![
        0xff, 0xd8, 0xff, 0xc0, 0x00, 0x08, 0x08, (height >> 8) as u8, height as u8,
        (width >> 8) as u8, width as u8, 0x01, 0xff, 0xd9,
    ]
}

fn read_request(stream: &mut std::net::TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("timeout");
    let mut bytes = Vec::new();
    let mut buffer = [0; 4096];
    loop {
        let count = stream.read(&mut buffer).expect("read");
        assert_ne!(count, 0);
        bytes.extend_from_slice(&buffer[..count]);
        if let Some(split) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&bytes[..split]);
            let length = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|v| v.trim().parse::<usize>().ok())
                })
                .unwrap_or(0);
            if bytes.len() >= split + 4 + length {
                return String::from_utf8(bytes).expect("request text");
            }
        }
    }
}

fn reply_bytes(
    stream: &mut std::net::TcpStream,
    status: u16,
    mime: &str,
    body: &[u8],
) {
    write!(
        stream,
        "HTTP/1.1 {status} Fixture\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .expect("status line");
    stream.write_all(body).expect("body");
}

#[tokio::test]
async fn a_wrong_size_is_invalid_before_anything_is_sent() {
    let executor = BflExecutor::new(
        BflConfig::loopback_fixture("127.0.0.1:9".parse().expect("loopback")).expect("config"),
        "fixture-key",
        bounds(),
        Duration::from_secs(5),
    )
    .expect("executor");
    let mut bad = portrait("fictional adult adventurer");
    bad.width = 512;
    bad.height = 512;
    assert_eq!(
        executor.edit(&bad, Duration::from_secs(5)).await.map(|_| ()),
        Err(ApiError::InvalidRequest)
    );
}

#[tokio::test]
async fn a_moderated_job_sends_once_and_never_downloads() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("submit");
        let request = read_request(&mut stream);
        assert!(request.starts_with("POST /v1/flux-3-image "));
        let body = serde_json::json!({
            "polling_url": format!("http://{address}/v1/get_result?id=job-1"),
        })
        .to_string();
        reply_bytes(&mut stream, 200, "application/json", body.as_bytes());
        let (mut stream, _) = listener.accept().expect("poll");
        let _ = read_request(&mut stream);
        reply_bytes(
            &mut stream,
            503,
            "application/json",
            b"{\"status\":\"Content Moderated\"}",
        );
    });
    let executor = BflExecutor::new(
        BflConfig::loopback_fixture(address).expect("config"),
        "fixture-key",
        bounds(),
        Duration::from_secs(5),
    )
    .expect("executor");
    assert_eq!(
        executor
            .edit(&portrait("fictional adult adventurer"), Duration::from_secs(5))
            .await
            .map(|_| ()),
        Err(ApiError::Refused)
    );
    server.join().expect("server");
}

#[tokio::test]
async fn an_untrusted_poll_host_never_receives_credentials() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("submit");
        let _ = read_request(&mut stream);
        reply_bytes(
            &mut stream,
            200,
            "application/json",
            b"{\"polling_url\":\"https://evil.test/v1/get_result?id=x\"}",
        );
    });
    let executor = BflExecutor::new(
        BflConfig::loopback_fixture(address).expect("config"),
        "fixture-key",
        bounds(),
        Duration::from_secs(5),
    )
    .expect("executor");
    assert_eq!(
        executor
            .edit(&portrait("fictional adult adventurer"), Duration::from_secs(5))
            .await
            .map(|_| ()),
        Err(ApiError::Output)
    );
    server.join().expect("server");
}

#[tokio::test]
async fn submit_poll_and_download_failures_keep_their_stage() {
    for (expected, response_status) in [("submit", 402), ("poll", 429)] {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        let address = listener.local_addr().expect("address");
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("submit");
            assert!(read_request(&mut stream).starts_with("POST "));
            if expected == "submit" {
                reply_bytes(&mut stream, response_status, "application/json", b"{}");
                return;
            }
            let body = serde_json::json!({
                "polling_url": format!("http://{address}/v1/get_result?id=fixture"),
            })
            .to_string();
            reply_bytes(&mut stream, 200, "application/json", body.as_bytes());
            let (mut stream, _) = listener.accept().expect("poll");
            assert!(read_request(&mut stream).starts_with("GET /v1/get_result"));
            reply_bytes(&mut stream, response_status, "application/json", b"{}");
        });
        let executor = BflExecutor::new(
            BflConfig::loopback_fixture(address).expect("config"),
            "fixture-key",
            bounds(),
            Duration::from_secs(5),
        )
        .expect("executor");
        let failure = executor
            .edit_diagnosed(&portrait("fictional adult adventurer"), Duration::from_secs(5))
            .await
            .expect_err("diagnosed failure");
        assert_eq!(failure.stage, expected);
        server.join().expect("server");
    }
}

#[tokio::test]
async fn a_ready_jpeg_download_preserves_actual_bytes_without_a_credential() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("submit");
        let _ = read_request(&mut stream);
        let body = serde_json::json!({
            "polling_url": format!("http://{address}/v1/get_result?id=job-1"),
            "cost": 7,
        })
        .to_string();
        reply_bytes(&mut stream, 200, "application/json", body.as_bytes());
        let (mut stream, _) = listener.accept().expect("poll");
        let _ = read_request(&mut stream);
        reply_bytes(
            &mut stream,
            200,
            "application/json",
            format!(
                "{{\"status\":\"Ready\",\"result\":{{\"sample\":\"http://{address}/image.jpg\"}}}}"
            )
            .as_bytes(),
        );
        let (mut stream, _) = listener.accept().expect("download");
        let request = read_request(&mut stream).to_ascii_lowercase();
        assert!(!request.contains("fixture-key"));
        assert!(!request.contains("x-key:"));
        assert!(!request.contains("authorization:"));
        reply_bytes(&mut stream, 200, "image/jpeg", &minimal_jpeg(1024, 1536));
    });
    let executor = BflExecutor::new(
        BflConfig::loopback_fixture(address).expect("config"),
        "fixture-key",
        bounds(),
        Duration::from_secs(5),
    )
    .expect("executor");
    let success = executor
        .edit(&portrait("fictional adult adventurer"), Duration::from_secs(5))
        .await
        .expect("image");
    assert_eq!((success.image.width, success.image.height), (1024, 1536));
    assert_eq!(success.image.bytes, minimal_jpeg(1024, 1536));
    assert_eq!(success.credits.as_deref(), Some("7"));
    server.join().expect("server");
}
