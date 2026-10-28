//! Failure-focused loopback tests for the private fal backend.
//!
//! No live calls. Generation and alpha removal are exercised as explicit
//! separately metered stages; a transparent edit must carry both receipts.

use super::fal::{FalConfig, FalExecutor};
use super::{ApiError, ImageBackground, ImageEdit, ImageFormat, ImageModeration, ImageQuality};
use super::{ImageEditModel, OperationalBounds};
use df_providers::NativeHttpRefusal;
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::atomic::{AtomicUsize, Ordering},
    sync::Arc,
    time::Duration,
};

fn bounds() -> OperationalBounds {
    OperationalBounds::compatibility_defaults()
}

fn edit<'a>(
    prompt: &'a str,
    background: ImageBackground,
    output: ImageFormat,
) -> ImageEdit<'a> {
    ImageEdit {
        model: ImageEditModel::Gpt25Flare,
        prompt,
        references: &[],
        width: 1024,
        height: 1024,
        quality: ImageQuality::High,
        background,
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
    let mut buffer = [0; 8192];
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

fn data_uri_jpeg(width: u16, height: u16) -> String {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    format!(
        "data:image/jpeg;base64,{}",
        STANDARD.encode(minimal_jpeg(width, height))
    )
}

#[tokio::test]
async fn an_opaque_generation_is_one_stage_with_its_own_receipt() {
    let uri = data_uri_jpeg(1024, 1024);
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let posts = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&posts);
    let server = std::thread::spawn(move || {
        let body = serde_json::json!({
            "request_id": "req-1",
            "status_url": format!("https://queue.fal.run/x/requests/req-1/status"),
            "response_url": format!("https://queue.fal.run/x/requests/req-1"),
        })
        .to_string();
        let (mut stream, _) = listener.accept().expect("submit");
        let _ = read_request(&mut stream);
        counted.fetch_add(1, Ordering::SeqCst);
        reply(&mut stream, 200, body.as_bytes());
        let (mut stream, _) = listener.accept().expect("status");
        let _ = read_request(&mut stream);
        reply(&mut stream, 200, b"{\"status\":\"COMPLETED\"}");
        let (mut stream, _) = listener.accept().expect("result");
        let _ = read_request(&mut stream);
        let result = serde_json::json!({"images": [{"url": uri}]}).to_string();
        reply(&mut stream, 200, result.as_bytes());
    });
    let executor = FalExecutor::new(
        FalConfig::loopback_fixture(address).expect("config"),
        "fixture-key",
        bounds(),
        Duration::from_secs(5),
    )
    .expect("executor");
    let outcome = executor
        .edit(
            &edit("a room", ImageBackground::Opaque, ImageFormat::Jpeg),
            Duration::from_secs(5),
        )
        .await
        .expect("outcome");
    assert_eq!((outcome.image.width, outcome.image.height), (1024, 1024));
    assert!(outcome.alpha.is_none());
    assert_eq!(outcome.generation.stage.endpoint, super::fal::FLUX_ULTRA);
    assert_eq!(posts.load(Ordering::SeqCst), 1);
    server.join().expect("server");
}

#[tokio::test]
async fn a_transparent_edit_carries_generation_and_alpha_receipts() {
    let generated = data_uri_jpeg(64, 64);
    let cutout = data_uri_jpeg(64, 64);
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let server = std::thread::spawn(move || {
        let submit = serde_json::json!({
            "request_id": "gen-1",
            "status_url": "https://queue.fal.run/x/requests/gen-1/status",
            "response_url": "https://queue.fal.run/x/requests/gen-1",
        })
        .to_string();
        let (mut stream, _) = listener.accept().expect("gen submit");
        let _ = read_request(&mut stream);
        reply(&mut stream, 200, submit.as_bytes());
        let (mut stream, _) = listener.accept().expect("gen status");
        let _ = read_request(&mut stream);
        reply(&mut stream, 200, b"{\"status\":\"COMPLETED\"}");
        let (mut stream, _) = listener.accept().expect("gen result");
        let _ = read_request(&mut stream);
        reply(
            &mut stream,
            200,
            serde_json::json!({"images": [{"url": generated}]}).to_string().as_bytes(),
        );
        let alpha_submit = serde_json::json!({
            "request_id": "alpha-1",
            "status_url": "https://queue.fal.run/x/requests/alpha-1/status",
            "response_url": "https://queue.fal.run/x/requests/alpha-1",
        })
        .to_string();
        let (mut stream, _) = listener.accept().expect("alpha submit");
        let _ = read_request(&mut stream);
        reply(&mut stream, 200, alpha_submit.as_bytes());
        let (mut stream, _) = listener.accept().expect("alpha status");
        let _ = read_request(&mut stream);
        reply(&mut stream, 200, b"{\"status\":\"COMPLETED\"}");
        let (mut stream, _) = listener.accept().expect("alpha result");
        let _ = read_request(&mut stream);
        reply(
            &mut stream,
            200,
            serde_json::json!({"images": [{"url": cutout}]}).to_string().as_bytes(),
        );
    });
    let executor = FalExecutor::new(
        FalConfig::loopback_fixture(address).expect("config"),
        "fixture-key",
        bounds(),
        Duration::from_secs(10),
    )
    .expect("executor");
    let outcome = executor
        .edit(
            &edit("a figure", ImageBackground::Transparent, ImageFormat::Png),
            Duration::from_secs(10),
        )
        .await
        .expect("outcome");
    assert!(outcome.alpha.is_some());
    assert_eq!(
        outcome.alpha.as_ref().expect("alpha").stage.endpoint,
        super::fal::BACKGROUND_REMOVAL
    );
    assert_eq!(outcome.generation.stage.endpoint, super::fal::FLUX_ULTRA);
    server.join().expect("server");
}

#[tokio::test]
async fn content_refusal_is_terminal_and_never_starts_a_second_stage() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("submit");
        let _ = read_request(&mut stream);
        reply(&mut stream, 422, b"{\"detail\":\"content_policy violated\"}");
    });
    let executor = FalExecutor::new(
        FalConfig::loopback_fixture(address).expect("config"),
        "fixture-key",
        bounds(),
        Duration::from_secs(5),
    )
    .expect("executor");
    assert_eq!(
        executor
            .edit(
                &edit("a room", ImageBackground::Opaque, ImageFormat::Jpeg),
                Duration::from_secs(5)
            )
            .await
            .map(|_| ()),
        Err(ApiError::Refused)
    );
    server.join().expect("server");
}

#[tokio::test]
async fn an_endless_queue_is_an_unknown_deadline_without_resubmission() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let submits = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&submits);
    let server = std::thread::spawn(move || {
        let body = serde_json::json!({
            "request_id": "req-wait",
            "status_url": "https://queue.fal.run/x/requests/req-wait/status",
            "response_url": "https://queue.fal.run/x/requests/req-wait",
        })
        .to_string();
        let (mut stream, _) = listener.accept().expect("submit");
        let _ = read_request(&mut stream);
        counted.fetch_add(1, Ordering::SeqCst);
        reply(&mut stream, 200, body.as_bytes());
        let (mut stream, _) = listener.accept().expect("status");
        let _ = read_request(&mut stream);
        reply(&mut stream, 200, b"{\"status\":\"IN_QUEUE\"}");
    });
    let executor = FalExecutor::new(
        FalConfig::loopback_fixture(address).expect("config"),
        "fixture-key",
        bounds(),
        Duration::from_secs(5),
    )
    .expect("executor");
    assert_eq!(
        executor
            .edit(
                &edit("a room", ImageBackground::Opaque, ImageFormat::Jpeg),
                Duration::from_millis(60)
            )
            .await
            .map(|_| ()),
        Err(ApiError::Transport(NativeHttpRefusal::Deadline))
    );
    assert_eq!(submits.load(Ordering::SeqCst), 1);
    server.join().expect("server");
}

#[tokio::test]
async fn a_hosted_result_url_is_an_output_failure_not_a_fetch() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let server = std::thread::spawn(move || {
        let body = serde_json::json!({
            "request_id": "req-h",
            "status_url": "https://queue.fal.run/x/requests/req-h/status",
            "response_url": "https://queue.fal.run/x/requests/req-h",
        })
        .to_string();
        let (mut stream, _) = listener.accept().expect("submit");
        let _ = read_request(&mut stream);
        reply(&mut stream, 200, body.as_bytes());
        let (mut stream, _) = listener.accept().expect("status");
        let _ = read_request(&mut stream);
        reply(&mut stream, 200, b"{\"status\":\"COMPLETED\"}");
        let (mut stream, _) = listener.accept().expect("result");
        let _ = read_request(&mut stream);
        reply(
            &mut stream,
            200,
            b"{\"images\":[{\"url\":\"https://v3.fal.media/files/x.jpg\"}]}",
        );
    });
    let executor = FalExecutor::new(
        FalConfig::loopback_fixture(address).expect("config"),
        "fixture-key",
        bounds(),
        Duration::from_secs(5),
    )
    .expect("executor");
    assert_eq!(
        executor
            .edit(
                &edit("a room", ImageBackground::Opaque, ImageFormat::Jpeg),
                Duration::from_secs(5)
            )
            .await
            .map(|_| ()),
        Err(ApiError::Output)
    );
    server.join().expect("server");
}
