//! Failure-focused loopback tests for the private OpenAI backend.
//!
//! No live calls. Each fixture speaks minimal HTTP on loopback and asserts
//! the single-call contract: no retry, reword or reroute.

use super::openai::{OpenAiConfig, OpenAiExecutor};
use super::{ApiError, ImageBackground, ImageEdit, ImageFormat, ImageModeration, ImageQuality};
use super::{ImageEditModel, OperationalBounds};
use df_providers::NativeHttpRefusal;
use std::{
    io::{Read, Write},
    net::TcpListener,
    time::Duration,
};

fn bounds() -> OperationalBounds {
    OperationalBounds::compatibility_defaults()
}

fn edit<'a>(prompt: &'a str, output: ImageFormat) -> ImageEdit<'a> {
    ImageEdit {
        model: ImageEditModel::Gpt25Flare,
        prompt,
        references: &[],
        width: 1024,
        height: 1024,
        quality: ImageQuality::Medium,
        background: ImageBackground::Opaque,
        output,
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

fn reply(stream: &mut std::net::TcpStream, status: u16, body: &[u8]) {
    write!(
        stream,
        "HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .expect("status line");
    stream.write_all(body).expect("body");
}

#[tokio::test]
async fn an_empty_prompt_is_invalid_before_anything_is_sent() {
    let executor = OpenAiExecutor::new(
        OpenAiConfig::loopback_fixture("127.0.0.1:9".parse().expect("loopback")).expect("config"),
        "fixture-key",
        bounds(),
        Duration::from_secs(5),
    )
    .expect("executor");
    assert_eq!(
        executor
            .edit(&edit("", ImageFormat::Jpeg), Duration::from_secs(5))
            .await,
        Err(ApiError::InvalidRequest)
    );
}

#[tokio::test]
async fn a_moderation_block_is_a_terminal_refusal_without_retry() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let request = read_request(&mut stream);
        assert!(request.starts_with("POST /v1/images/generations "));
        reply(&mut stream, 400, b"{\"error\":{\"code\":\"moderation_blocked\"}}");
    });
    let executor = OpenAiExecutor::new(
        OpenAiConfig::loopback_fixture(address).expect("config"),
        "fixture-key",
        bounds(),
        Duration::from_secs(5),
    )
    .expect("executor");
    assert_eq!(
        executor
            .edit(&edit("a tavern at dusk", ImageFormat::Jpeg), Duration::from_secs(5))
            .await,
        Err(ApiError::Refused)
    );
    server.join().expect("server");
}

#[tokio::test]
async fn quota_exhaustion_is_distinct_from_a_retryable_rate_limit() {
    for (body, expected) in [
        (
            "{\"error\":{\"code\":\"insufficient_quota\"}}",
            ApiError::QuotaExhausted,
        ),
        ("{\"error\":{\"code\":\"rate_limit\"}}", ApiError::Provider(429)),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        let address = listener.local_addr().expect("address");
        let owned = body.to_owned();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let _ = read_request(&mut stream);
            reply(&mut stream, 429, owned.as_bytes());
        });
        let executor = OpenAiExecutor::new(
            OpenAiConfig::loopback_fixture(address).expect("config"),
            "fixture-key",
            bounds(),
            Duration::from_secs(5),
        )
        .expect("executor");
        assert_eq!(
            executor
                .edit(&edit("a tavern at dusk", ImageFormat::Jpeg), Duration::from_secs(5))
                .await,
            Err(expected)
        );
        server.join().expect("server");
    }
}

#[tokio::test]
async fn a_success_answer_without_an_image_is_a_refusal_not_an_output_guess() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let _ = read_request(&mut stream);
        reply(&mut stream, 200, b"{\"data\":[]}");
    });
    let executor = OpenAiExecutor::new(
        OpenAiConfig::loopback_fixture(address).expect("config"),
        "fixture-key",
        bounds(),
        Duration::from_secs(5),
    )
    .expect("executor");
    assert_eq!(
        executor
            .edit(&edit("a tavern at dusk", ImageFormat::Jpeg), Duration::from_secs(5))
            .await,
        Err(ApiError::Refused)
    );
    server.join().expect("server");
}

#[tokio::test]
async fn a_result_of_another_size_is_an_output_failure() {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    let other = STANDARD.encode(minimal_jpeg(512, 512));
    let body = serde_json::json!({"data":[{"b64_json": other}]}).to_string();
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let _ = read_request(&mut stream);
        reply(&mut stream, 200, body.as_bytes());
    });
    let executor = OpenAiExecutor::new(
        OpenAiConfig::loopback_fixture(address).expect("config"),
        "fixture-key",
        bounds(),
        Duration::from_secs(5),
    )
    .expect("executor");
    assert_eq!(
        executor
            .edit(&edit("a tavern at dusk", ImageFormat::Jpeg), Duration::from_secs(5))
            .await,
        Err(ApiError::Output)
    );
    server.join().expect("server");
}

#[tokio::test]
async fn an_actual_jpeg_result_preserves_bytes_dimensions_and_usage() {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    let bytes = minimal_jpeg(1024, 1024);
    let encoded = STANDARD.encode(&bytes);
    let body = serde_json::json!({
        "data": [{"b64_json": encoded}],
        "usage": {"input_tokens_details": {"text_tokens": 7, "image_tokens": 3}, "output_tokens": 11}
    })
    .to_string();
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let _ = read_request(&mut stream);
        reply(&mut stream, 200, body.as_bytes());
    });
    let executor = OpenAiExecutor::new(
        OpenAiConfig::loopback_fixture(address).expect("config"),
        "fixture-key",
        bounds(),
        Duration::from_secs(5),
    )
    .expect("executor");
    let image = executor
        .edit(&edit("a tavern at dusk", ImageFormat::Jpeg), Duration::from_secs(5))
        .await
        .expect("image");
    assert_eq!(image.bytes, bytes);
    assert_eq!((image.width, image.height), (1024, 1024));
    assert_eq!(image.usage.micro_dollars(), 7 * 5 + 3 * 8 + 11 * 30);
    server.join().expect("server");
}

#[tokio::test]
async fn a_slow_supplier_hits_the_deadline_and_stays_unknown() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let _ = read_request(&mut stream);
        std::thread::sleep(Duration::from_millis(400));
        reply(&mut stream, 200, b"{\"data\":[]}");
    });
    let executor = OpenAiExecutor::new(
        OpenAiConfig::loopback_fixture(address).expect("config"),
        "fixture-key",
        bounds(),
        Duration::from_secs(5),
    )
    .expect("executor");
    assert_eq!(
        executor
            .edit(
                &edit("a tavern at dusk", ImageFormat::Jpeg),
                Duration::from_millis(50)
            )
            .await,
        Err(ApiError::Transport(NativeHttpRefusal::Deadline))
    );
    server.join().expect("server");
}
