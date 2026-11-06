//! Failure-focused loopback tests for the private fal backend.
//!
//! No live calls. Generation and alpha removal are exercised as explicit
//! separately metered stages; a transparent edit must carry both receipts.
//! Fixtures speak realistic queue routes on the loopback origin and return
//! actual encoder bytes so output facts are exact. Every fixture accept and
//! join is bounded so an early refusal never hangs the process.

use super::fal::{FalConfig, FalExecutor};
use super::{ApiError, ImageBackground, ImageEdit, ImageFormat, ImageModeration, ImageQuality};
use super::{ImageEditModel, OperationalBounds};
use df_providers::NativeHttpCause;
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::Arc,
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, Instant},
};

fn bounds() -> OperationalBounds {
    OperationalBounds::compatibility_defaults()
}

fn edit<'a>(prompt: &'a str, background: ImageBackground, output: ImageFormat) -> ImageEdit<'a> {
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

fn encode_jpeg(width: u32, height: u32) -> Vec<u8> {
    let picture = image::RgbImage::from_fn(width, height, |x, y| {
        image::Rgb([(x % 251) as u8, (y % 251) as u8, 128])
    });
    let mut cursor = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(picture)
        .write_to(&mut cursor, image::ImageFormat::Jpeg)
        .expect("encode jpeg");
    cursor.into_inner()
}

fn encode_png_opaque(width: u32, height: u32) -> Vec<u8> {
    let picture = image::RgbImage::from_fn(width, height, |x, y| {
        image::Rgb([(x % 251) as u8, (y % 251) as u8, 64])
    });
    let mut cursor = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(picture)
        .write_to(&mut cursor, image::ImageFormat::Png)
        .expect("encode png");
    cursor.into_inner()
}

fn encode_png_transparent(width: u32, height: u32) -> Vec<u8> {
    let picture = image::RgbaImage::from_fn(width, height, |x, y| {
        image::Rgba([(x % 251) as u8, (y % 251) as u8, 64, 255])
    });
    let mut cursor = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(picture)
        .write_to(&mut cursor, image::ImageFormat::Png)
        .expect("encode png");
    cursor.into_inner()
}

fn accept_next(listener: &TcpListener, timeout: Duration) -> std::net::TcpStream {
    let deadline = Instant::now() + timeout;
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                stream
                    .set_nonblocking(false)
                    .expect("blocking fixture stream");
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .expect("timeout");
                stream
                    .set_write_timeout(Some(Duration::from_secs(5)))
                    .expect("timeout");
                return stream;
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    panic!("fixture accept timed out");
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("fixture accept failed: {error}"),
        }
    }
}

fn join_bounded(server: std::thread::JoinHandle<()>, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while !server.is_finished() {
        if Instant::now() >= deadline {
            panic!("fixture server did not finish");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    server.join().expect("server");
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

fn data_uri_jpeg(bytes: &[u8]) -> String {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    format!("data:image/jpeg;base64,{}", STANDARD.encode(bytes))
}

fn data_uri_png(bytes: &[u8]) -> String {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    format!("data:image/png;base64,{}", STANDARD.encode(bytes))
}

#[tokio::test]
async fn an_opaque_generation_is_one_stage_with_its_own_receipt() {
    let image = encode_jpeg(1024, 1024);
    let uri = data_uri_jpeg(&image);
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    listener.set_nonblocking(true).expect("nonblocking");
    let address = listener.local_addr().expect("address");
    let posts = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&posts);
    let server = std::thread::spawn(move || {
        let body = serde_json::json!({
            "request_id": "req-1",
            "status_url": "https://queue.fal.run/fal-ai/flux-pro/v1.1-ultra/requests/req-1/status",
            "response_url": "https://queue.fal.run/fal-ai/flux-pro/v1.1-ultra/requests/req-1/response",
        })
        .to_string();
        let mut stream = accept_next(&listener, Duration::from_secs(10));
        let _ = read_request(&mut stream);
        counted.fetch_add(1, Ordering::SeqCst);
        reply(&mut stream, 200, body.as_bytes());
        let mut stream = accept_next(&listener, Duration::from_secs(10));
        let _ = read_request(&mut stream);
        reply(&mut stream, 200, b"{\"status\":\"COMPLETED\"}");
        let mut stream = accept_next(&listener, Duration::from_secs(10));
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
    assert_eq!(outcome.image.bytes, image);
    assert_eq!(outcome.image.format, ImageFormat::Jpeg);
    assert!(!outcome.image.has_alpha_channel);
    assert!(outcome.alpha.is_none());
    assert_eq!(outcome.generation.stage.endpoint, super::fal::FLUX_ULTRA);
    assert_eq!(
        outcome.generation.stage.request_id.as_deref(),
        Some("req-1")
    );
    assert_eq!(outcome.generation.stage.http_status, Some(200));
    assert_eq!(posts.load(Ordering::SeqCst), 1);
    join_bounded(server, Duration::from_secs(10));
}

#[tokio::test]
async fn a_transparent_edit_carries_generation_and_alpha_receipts() {
    let generated = encode_png_opaque(1024, 1024);
    let cutout = encode_png_transparent(1024, 1024);
    let generated_uri = data_uri_png(&generated);
    let cutout_uri = data_uri_png(&cutout);
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    listener.set_nonblocking(true).expect("nonblocking");
    let address = listener.local_addr().expect("address");
    let server = std::thread::spawn(move || {
        let submit = serde_json::json!({
            "request_id": "gen-1",
            "status_url": "https://queue.fal.run/fal-ai/flux-pro/v1.1-ultra/requests/gen-1/status",
            "response_url": "https://queue.fal.run/fal-ai/flux-pro/v1.1-ultra/requests/gen-1/response",
        })
        .to_string();
        let mut stream = accept_next(&listener, Duration::from_secs(10));
        let _ = read_request(&mut stream);
        reply(&mut stream, 200, submit.as_bytes());
        let mut stream = accept_next(&listener, Duration::from_secs(10));
        let _ = read_request(&mut stream);
        reply(&mut stream, 200, b"{\"status\":\"COMPLETED\"}");
        let mut stream = accept_next(&listener, Duration::from_secs(10));
        let _ = read_request(&mut stream);
        reply(
            &mut stream,
            200,
            serde_json::json!({"images": [{"url": generated_uri}]})
                .to_string()
                .as_bytes(),
        );
        let alpha_submit = serde_json::json!({
            "request_id": "alpha-1",
            "status_url": "https://queue.fal.run/fal-ai/birefnet/v2/requests/alpha-1/status",
            "response_url": "https://queue.fal.run/fal-ai/birefnet/v2/requests/alpha-1/response",
        })
        .to_string();
        let mut stream = accept_next(&listener, Duration::from_secs(10));
        let _ = read_request(&mut stream);
        reply(&mut stream, 200, alpha_submit.as_bytes());
        let mut stream = accept_next(&listener, Duration::from_secs(10));
        let _ = read_request(&mut stream);
        reply(&mut stream, 200, b"{\"status\":\"COMPLETED\"}");
        let mut stream = accept_next(&listener, Duration::from_secs(10));
        let _ = read_request(&mut stream);
        reply(
            &mut stream,
            200,
            serde_json::json!({"images": [{"url": cutout_uri}]})
                .to_string()
                .as_bytes(),
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
    assert_eq!((outcome.image.width, outcome.image.height), (1024, 1024));
    assert_eq!(outcome.image.bytes, cutout);
    assert_eq!(outcome.image.format, ImageFormat::Png);
    assert!(outcome.image.has_alpha_channel);
    assert_eq!(outcome.generation.stage.endpoint, super::fal::FLUX_ULTRA);
    assert_eq!(
        outcome.generation.stage.request_id.as_deref(),
        Some("gen-1")
    );
    assert_eq!(outcome.generation.stage.http_status, Some(200));
    assert_eq!(
        outcome.alpha.as_ref().expect("alpha").stage.endpoint,
        super::fal::BACKGROUND_REMOVAL
    );
    assert_eq!(
        outcome
            .alpha
            .as_ref()
            .expect("alpha")
            .stage
            .request_id
            .as_deref(),
        Some("alpha-1")
    );
    assert_eq!(
        outcome.alpha.as_ref().expect("alpha").stage.http_status,
        Some(200)
    );
    join_bounded(server, Duration::from_secs(10));
}

#[tokio::test]
async fn content_refusal_is_terminal_and_never_starts_a_second_stage() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    listener.set_nonblocking(true).expect("nonblocking");
    let address = listener.local_addr().expect("address");
    let server = std::thread::spawn(move || {
        let mut stream = accept_next(&listener, Duration::from_secs(10));
        let _ = read_request(&mut stream);
        reply(
            &mut stream,
            422,
            b"{\"detail\":\"content_policy violated\"}",
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
        Err(ApiError::Refused)
    );
    join_bounded(server, Duration::from_secs(10));
}

#[tokio::test]
async fn an_endless_queue_is_an_unknown_deadline_without_resubmission() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    listener.set_nonblocking(true).expect("nonblocking");
    let address = listener.local_addr().expect("address");
    let submits = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&submits);
    let server = std::thread::spawn(move || {
        let body = serde_json::json!({
            "request_id": "req-wait",
            "status_url": "https://queue.fal.run/fal-ai/flux-pro/v1.1-ultra/requests/req-wait/status",
            "response_url": "https://queue.fal.run/fal-ai/flux-pro/v1.1-ultra/requests/req-wait/response",
        })
        .to_string();
        let mut stream = accept_next(&listener, Duration::from_secs(10));
        let _ = read_request(&mut stream);
        counted.fetch_add(1, Ordering::SeqCst);
        reply(&mut stream, 200, body.as_bytes());
        let mut stream = accept_next(&listener, Duration::from_secs(10));
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
        Err(ApiError::Transport(NativeHttpCause::Deadline))
    );
    assert_eq!(submits.load(Ordering::SeqCst), 1);
    join_bounded(server, Duration::from_secs(10));
}

#[tokio::test]
async fn a_hosted_result_url_is_an_output_failure_not_a_fetch() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    listener.set_nonblocking(true).expect("nonblocking");
    let address = listener.local_addr().expect("address");
    let stages = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&stages);
    let server = std::thread::spawn(move || {
        let body = serde_json::json!({
            "request_id": "req-h",
            "status_url": "https://queue.fal.run/fal-ai/flux-pro/v1.1-ultra/requests/req-h/status",
            "response_url": "https://queue.fal.run/fal-ai/flux-pro/v1.1-ultra/requests/req-h/response",
        })
        .to_string();
        let mut stream = accept_next(&listener, Duration::from_secs(10));
        let _ = read_request(&mut stream);
        counted.fetch_add(1, Ordering::SeqCst);
        reply(&mut stream, 200, body.as_bytes());
        let mut stream = accept_next(&listener, Duration::from_secs(10));
        let _ = read_request(&mut stream);
        counted.fetch_add(1, Ordering::SeqCst);
        reply(&mut stream, 200, b"{\"status\":\"COMPLETED\"}");
        let mut stream = accept_next(&listener, Duration::from_secs(10));
        let _ = read_request(&mut stream);
        counted.fetch_add(1, Ordering::SeqCst);
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
    assert_eq!(stages.load(Ordering::SeqCst), 3);
    join_bounded(server, Duration::from_secs(10));
}

#[tokio::test]
async fn submit_persist_recreate_resume_makes_one_post() {
    let image = encode_jpeg(1024, 1024);
    let uri = data_uri_jpeg(&image);
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    listener.set_nonblocking(true).expect("nonblocking");
    let address = listener.local_addr().expect("address");
    let posts = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&posts);
    let server = std::thread::spawn(move || {
        let body = serde_json::json!({
            "request_id": "req-resume-1",
            "status_url": "https://queue.fal.run/fal-ai/flux-pro/v1.1-ultra/requests/req-resume-1/status",
            "response_url": "https://queue.fal.run/fal-ai/flux-pro/v1.1-ultra/requests/req-resume-1/response",
        })
        .to_string();
        let mut stream = accept_next(&listener, Duration::from_secs(10));
        let request = read_request(&mut stream);
        assert!(request.starts_with("POST "));
        counted.fetch_add(1, Ordering::SeqCst);
        reply(&mut stream, 200, body.as_bytes());
        let mut stream = accept_next(&listener, Duration::from_secs(10));
        let request = read_request(&mut stream);
        assert!(request.starts_with("GET "));
        reply(&mut stream, 200, b"{\"status\":\"COMPLETED\"}");
        let mut stream = accept_next(&listener, Duration::from_secs(10));
        let request = read_request(&mut stream);
        assert!(request.starts_with("GET "));
        let result = serde_json::json!({"images": [{"url": uri}]}).to_string();
        reply(&mut stream, 200, result.as_bytes());
    });
    let submitter = FalExecutor::new(
        FalConfig::loopback_fixture(address).expect("config"),
        "fixture-key",
        bounds(),
        Duration::from_secs(5),
    )
    .expect("executor");
    let checkpoint = submitter
        .submit_generation(
            &edit("a room", ImageBackground::Opaque, ImageFormat::Jpeg),
            Duration::from_secs(5),
        )
        .await
        .expect("checkpoint");
    assert_eq!(checkpoint.request_id, "req-resume-1");
    let persisted = checkpoint.encode();
    let restored = super::fal::FalGenerationCheckpoint::decode(&persisted).expect("decode");
    assert_eq!(restored, checkpoint);
    let debug = format!("{restored:?}");
    assert!(!debug.contains("req-resume-1"));
    assert!(!debug.contains("status"));
    drop(submitter);
    let resumer = FalExecutor::new(
        FalConfig::loopback_fixture(address).expect("config"),
        "fixture-key",
        bounds(),
        Duration::from_secs(5),
    )
    .expect("executor");
    let (bytes, format, receipt) = resumer
        .resume_generation(&restored, Duration::from_secs(5))
        .await
        .expect("resume");
    assert_eq!(bytes, image);
    assert_eq!(format, ImageFormat::Jpeg);
    assert_eq!(receipt.stage.request_id.as_deref(), Some("req-resume-1"));
    assert_eq!(posts.load(Ordering::SeqCst), 1);
    join_bounded(server, Duration::from_secs(10));
}

#[tokio::test]
async fn alpha_submit_persist_recreate_resume_makes_one_post() {
    let generated = encode_png_opaque(1024, 1024);
    let cutout = encode_png_transparent(1024, 1024);
    let cutout_uri = data_uri_png(&cutout);
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    listener.set_nonblocking(true).expect("nonblocking");
    let address = listener.local_addr().expect("address");
    let posts = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&posts);
    let server = std::thread::spawn(move || {
        let body = serde_json::json!({
            "request_id": "alpha-resume-1",
            "status_url": "https://queue.fal.run/fal-ai/birefnet/v2/requests/alpha-resume-1/status",
            "response_url": "https://queue.fal.run/fal-ai/birefnet/v2/requests/alpha-resume-1/response",
        })
        .to_string();
        let mut stream = accept_next(&listener, Duration::from_secs(10));
        let request = read_request(&mut stream);
        assert!(request.starts_with("POST /fal-ai/birefnet/v2 "));
        counted.fetch_add(1, Ordering::SeqCst);
        reply(&mut stream, 200, body.as_bytes());
        let mut stream = accept_next(&listener, Duration::from_secs(10));
        let request = read_request(&mut stream);
        assert!(request.starts_with("GET "));
        reply(&mut stream, 200, b"{\"status\":\"COMPLETED\"}");
        let mut stream = accept_next(&listener, Duration::from_secs(10));
        let request = read_request(&mut stream);
        assert!(request.starts_with("GET "));
        reply(
            &mut stream,
            200,
            serde_json::json!({"images": [{"url": cutout_uri}]})
                .to_string()
                .as_bytes(),
        );
    });
    let submitter = FalExecutor::new(
        FalConfig::loopback_fixture(address).expect("config"),
        "fixture-key",
        bounds(),
        Duration::from_secs(5),
    )
    .expect("executor");
    let checkpoint = submitter
        .submit_alpha_removal(&generated, ImageFormat::Png, Duration::from_secs(5))
        .await
        .expect("checkpoint");
    assert_eq!(checkpoint.request_id, "alpha-resume-1");
    let persisted = checkpoint.encode();
    let restored = super::fal::FalAlphaCheckpoint::decode(&persisted).expect("decode");
    assert_eq!(restored, checkpoint);
    drop(submitter);
    let resumer = FalExecutor::new(
        FalConfig::loopback_fixture(address).expect("config"),
        "fixture-key",
        bounds(),
        Duration::from_secs(5),
    )
    .expect("executor");
    let (bytes, format, receipt) = resumer
        .resume_alpha_removal(&restored, Duration::from_secs(5))
        .await
        .expect("resume");
    assert_eq!(bytes, cutout);
    assert_eq!(format, ImageFormat::Png);
    assert_eq!(receipt.stage.request_id.as_deref(), Some("alpha-resume-1"));
    assert_eq!(posts.load(Ordering::SeqCst), 1);
    join_bounded(server, Duration::from_secs(10));
}

#[tokio::test]
async fn a_forged_checkpoint_is_rejected_before_any_egress() {
    let executor = FalExecutor::new(
        FalConfig::loopback_fixture("127.0.0.1:9".parse().expect("loopback")).expect("config"),
        "fixture-key",
        bounds(),
        Duration::from_secs(5),
    )
    .expect("executor");
    for forged in [
        "fal-generation-v2|http://127.0.0.1:9|fal-ai/flux-pro/v1.1-ultra|req-1|https://evil.test/x|/fal-ai/a/requests/req-1/response",
        "fal-generation-v2|http://127.0.0.1:9|fal-ai/flux-pro/v1.1-ultra|req-1|/fal-ai/a/requests/req-1/status?x=1|/fal-ai/a/requests/req-1/response",
        "fal-generation-v2|http://127.0.0.1:9|fal-ai/flux-pro/v1.1-ultra|other|/fal-ai/a/requests/req-1/status|/fal-ai/a/requests/req-1/response",
        "fal-generation-v2|http://127.0.0.1:9|fal-ai/flux-pro/v1.1-ultra|req-1|/fal-ai/a/requests/req-1/status|/fal-ai/a/requests/req-1/response|extra",
        "fal-generation-v1|fal-ai/flux-pro/v1.1-ultra|req-1|/fal-ai/a/requests/req-1/status|/fal-ai/a/requests/req-1/response",
        "not-a-checkpoint",
    ] {
        match super::fal::FalGenerationCheckpoint::decode(forged) {
            Err(ApiError::InvalidRequest) => {}
            Ok(checkpoint) => assert_eq!(
                executor
                    .resume_generation(&checkpoint, Duration::from_secs(2))
                    .await
                    .map(|_| ()),
                Err(ApiError::InvalidRequest)
            ),
            Err(other) => panic!("wrong refusal for {forged:?}: {other:?}"),
        }
    }
}

#[tokio::test]
async fn a_deadline_during_resume_never_resubmits() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    listener.set_nonblocking(true).expect("nonblocking");
    let address = listener.local_addr().expect("address");
    let posts = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&posts);
    let server = std::thread::spawn(move || {
        let body = serde_json::json!({
            "request_id": "req-slow",
            "status_url": "https://queue.fal.run/fal-ai/flux-pro/v1.1-ultra/requests/req-slow/status",
            "response_url": "https://queue.fal.run/fal-ai/flux-pro/v1.1-ultra/requests/req-slow/response",
        })
        .to_string();
        let mut stream = accept_next(&listener, Duration::from_secs(10));
        let request = read_request(&mut stream);
        assert!(request.starts_with("POST "));
        counted.fetch_add(1, Ordering::SeqCst);
        reply(&mut stream, 200, body.as_bytes());
        let mut stream = accept_next(&listener, Duration::from_secs(10));
        let _ = read_request(&mut stream);
        reply(&mut stream, 200, b"{\"status\":\"IN_QUEUE\"}");
    });
    let submitter = FalExecutor::new(
        FalConfig::loopback_fixture(address).expect("config"),
        "fixture-key",
        bounds(),
        Duration::from_secs(5),
    )
    .expect("executor");
    let checkpoint = submitter
        .submit_generation(
            &edit("a room", ImageBackground::Opaque, ImageFormat::Jpeg),
            Duration::from_secs(5),
        )
        .await
        .expect("checkpoint");
    drop(submitter);
    let resumer = FalExecutor::new(
        FalConfig::loopback_fixture(address).expect("config"),
        "fixture-key",
        bounds(),
        Duration::from_secs(5),
    )
    .expect("executor");
    assert_eq!(
        resumer
            .resume_generation(&checkpoint, Duration::from_millis(60))
            .await
            .map(|_| ()),
        Err(ApiError::Transport(NativeHttpCause::Deadline))
    );
    assert_eq!(posts.load(Ordering::SeqCst), 1);
    join_bounded(server, Duration::from_secs(10));
}

#[tokio::test]
async fn a_rejected_status_during_resume_never_resubmits() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    listener.set_nonblocking(true).expect("nonblocking");
    let address = listener.local_addr().expect("address");
    let posts = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&posts);
    let server = std::thread::spawn(move || {
        let body = serde_json::json!({
            "request_id": "req-bad",
            "status_url": "https://queue.fal.run/fal-ai/flux-pro/v1.1-ultra/requests/req-bad/status",
            "response_url": "https://queue.fal.run/fal-ai/flux-pro/v1.1-ultra/requests/req-bad/response",
        })
        .to_string();
        let mut stream = accept_next(&listener, Duration::from_secs(10));
        let request = read_request(&mut stream);
        assert!(request.starts_with("POST "));
        counted.fetch_add(1, Ordering::SeqCst);
        reply(&mut stream, 200, body.as_bytes());
        let mut stream = accept_next(&listener, Duration::from_secs(10));
        let _ = read_request(&mut stream);
        reply(&mut stream, 200, b"{\"status\":\"FAILED\"}");
    });
    let submitter = FalExecutor::new(
        FalConfig::loopback_fixture(address).expect("config"),
        "fixture-key",
        bounds(),
        Duration::from_secs(5),
    )
    .expect("executor");
    let checkpoint = submitter
        .submit_generation(
            &edit("a room", ImageBackground::Opaque, ImageFormat::Jpeg),
            Duration::from_secs(5),
        )
        .await
        .expect("checkpoint");
    drop(submitter);
    let resumer = FalExecutor::new(
        FalConfig::loopback_fixture(address).expect("config"),
        "fixture-key",
        bounds(),
        Duration::from_secs(5),
    )
    .expect("executor");
    assert_eq!(
        resumer
            .resume_generation(&checkpoint, Duration::from_secs(5))
            .await
            .map(|_| ()),
        Err(ApiError::Provider(500))
    );
    assert_eq!(posts.load(Ordering::SeqCst), 1);
    join_bounded(server, Duration::from_secs(10));
}

#[test]
fn configured_slugs_and_operator_urls_are_validated() {
    assert!(FalConfig::operator("https://queue.fal.run").is_ok());
    assert_eq!(
        FalConfig::operator("https://user@example.test").map(|_| ()),
        Err(ApiError::InvalidRequest)
    );
    assert_eq!(
        FalConfig::operator("https://queue.example.test/search?q=1").map(|_| ()),
        Err(ApiError::InvalidRequest)
    );
    assert_eq!(
        FalConfig::operator("https://queue.example.test/#frag").map(|_| ()),
        Err(ApiError::InvalidRequest)
    );
    assert_eq!(
        FalConfig::operator("https://queue.example.test/\x07").map(|_| ()),
        Err(ApiError::InvalidRequest)
    );
    assert_eq!(
        FalConfig::operator("http://192.0.2.1/").map(|_| ()),
        Err(ApiError::InvalidRequest)
    );
    let address: std::net::SocketAddr = "127.0.0.1:9".parse().expect("loopback");
    assert!(
        FalConfig::loopback_fixture(address)
            .expect("config")
            .with_models(
                "fal-ai/custom-gen/v9",
                "fal-ai/custom-edit/v9",
                "fal-ai/custom-alpha/v9"
            )
            .is_ok()
    );
    assert_eq!(
        FalConfig::loopback_fixture(address)
            .expect("config")
            .with_models(
                "fal-ai/custom-gen/v9",
                "bad slug?",
                "fal-ai/custom-alpha/v9"
            )
            .map(|_| ()),
        Err(ApiError::InvalidRequest)
    );
    assert_eq!(
        super::ImageEditModel::parse_model_id("gpt-image-2.5-custom")
            .map(|model| model.model_id().to_owned()),
        Ok("gpt-image-2.5-custom".to_owned())
    );
    assert_eq!(
        super::ImageEditModel::parse_model_id("bad model?").map(|_| ()),
        Err(ApiError::InvalidRequest)
    );
    let debug = format!(
        "{:?}",
        FalConfig::operator("https://queue.fal.run").expect("config")
    );
    assert!(!debug.contains("queue.fal.run"));
}

#[tokio::test]
async fn a_custom_generation_slug_flows_through_the_submit_path() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    listener.set_nonblocking(true).expect("nonblocking");
    let address = listener.local_addr().expect("address");
    let server = std::thread::spawn(move || {
        let mut stream = accept_next(&listener, Duration::from_secs(10));
        let request = read_request(&mut stream);
        assert!(request.starts_with("POST /fal-ai/custom-gen/v9 "));
        let body = serde_json::json!({
            "request_id": "req-custom",
            "status_url": "https://queue.fal.run/fal-ai/custom-gen/v9/requests/req-custom/status",
            "response_url": "https://queue.fal.run/fal-ai/custom-gen/v9/requests/req-custom/response",
        })
        .to_string();
        reply(&mut stream, 200, body.as_bytes());
    });
    let executor = FalExecutor::new(
        FalConfig::loopback_fixture(address)
            .expect("config")
            .with_models(
                "fal-ai/custom-gen/v9",
                "fal-ai/flux-pro/kontext/max/multi",
                "fal-ai/birefnet/v2",
            )
            .expect("models"),
        "fixture-key",
        bounds(),
        Duration::from_secs(5),
    )
    .expect("executor");
    let checkpoint = executor
        .submit_generation(
            &edit("a room", ImageBackground::Opaque, ImageFormat::Jpeg),
            Duration::from_secs(5),
        )
        .await
        .expect("checkpoint");
    assert_eq!(checkpoint.endpoint, "fal-ai/custom-gen/v9");
    join_bounded(server, Duration::from_secs(10));
}

#[test]
fn operator_origin_uses_structural_host_validation_not_prefix() {
    assert_eq!(
        super::fal::FalConfig::operator("http://127.example.test/").map(|_| ()),
        Err(ApiError::InvalidRequest)
    );
    assert_eq!(
        super::bfl::BflConfig::operator("http://127.example.test").map(|_| ()),
        Err(ApiError::InvalidRequest)
    );
    assert_eq!(
        super::openai::OpenAiConfig::operator(
            "http://127.example.test/v1/images/edits",
            "http://127.example.test/v1/images/generations"
        )
        .map(|_| ()),
        Err(ApiError::InvalidRequest)
    );
    assert!(super::fal::FalConfig::operator("http://[::1]:18080").is_ok());
    assert!(super::fal::FalConfig::operator("https://queue.fal.run").is_ok());
    assert!(super::fal::FalConfig::operator("https://queue.fal.run:8443").is_err());
    for bad in [
        "https://user@example.test",
        "https://queue.example.test/search?q=1",
        "https://queue.example.test/#frag",
        "https://queue.example.test/\x07",
        "https://queue.example.test/%2e",
        "https://queue.example.test/a%2Fb",
        "https://queue.example.test/..",
        "http://192.0.2.1/",
        "http://127.example.test:8080/",
        "http://[::1",
        "https://:8443",
        "https://queue.example.test:0",
        "https://queue.example.test:99999",
        "ftp://queue.example.test/",
    ] {
        assert_eq!(
            super::fal::FalConfig::operator(bad).map(|_| ()),
            Err(ApiError::InvalidRequest),
            "{bad:?}"
        );
    }
    let debug = format!(
        "{:?}",
        super::fal::FalConfig::operator("https://queue.fal.run").expect("config")
    );
    assert!(!debug.contains("queue.fal.run"));
}

#[tokio::test]
async fn a_generation_checkpoint_never_decodes_or_resumes_an_alpha_receipt() {
    let executor = FalExecutor::new(
        FalConfig::loopback_fixture("127.0.0.1:9".parse().expect("loopback")).expect("config"),
        "fixture-key",
        bounds(),
        Duration::from_secs(5),
    )
    .expect("executor");
    let alpha_encoded = super::fal::FalAlphaCheckpoint {
        origin: "http://127.0.0.1:9".to_owned(),
        endpoint: super::fal::BACKGROUND_REMOVAL.to_owned(),
        request_id: "alpha-1".to_owned(),
        status_path: "/fal-ai/birefnet/v2/requests/alpha-1/status".to_owned(),
        result_path: "/fal-ai/birefnet/v2/requests/alpha-1/response".to_owned(),
    }
    .encode();
    assert_eq!(
        super::fal::FalGenerationCheckpoint::decode(&alpha_encoded),
        Err(ApiError::InvalidRequest)
    );
    let generation_encoded = super::fal::FalGenerationCheckpoint {
        origin: "http://127.0.0.1:9".to_owned(),
        endpoint: super::fal::FLUX_ULTRA.to_owned(),
        request_id: "gen-1".to_owned(),
        status_path: "/fal-ai/flux-pro/v1.1-ultra/requests/gen-1/status".to_owned(),
        result_path: "/fal-ai/flux-pro/v1.1-ultra/requests/gen-1/response".to_owned(),
    }
    .encode();
    assert_eq!(
        super::fal::FalAlphaCheckpoint::decode(&generation_encoded),
        Err(ApiError::InvalidRequest)
    );
    let alpha_as_generation = super::fal::FalGenerationCheckpoint {
        origin: "http://127.0.0.1:9".to_owned(),
        endpoint: super::fal::BACKGROUND_REMOVAL.to_owned(),
        request_id: "alpha-1".to_owned(),
        status_path: "/fal-ai/birefnet/v2/requests/alpha-1/status".to_owned(),
        result_path: "/fal-ai/birefnet/v2/requests/alpha-1/response".to_owned(),
    };
    assert_eq!(
        executor
            .resume_generation(&alpha_as_generation, Duration::from_secs(2))
            .await
            .map(|_| ()),
        Err(ApiError::InvalidRequest)
    );
}

#[tokio::test]
async fn a_generation_checkpoint_rebound_to_another_model_or_origin_is_refused() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    listener.set_nonblocking(true).expect("nonblocking");
    let address = listener.local_addr().expect("address");
    let server = std::thread::spawn(move || {
        let body = serde_json::json!({
            "request_id": "req-rebind-1",
            "status_url": "https://queue.fal.run/fal-ai/flux-pro/v1.1-ultra/requests/req-rebind-1/status",
            "response_url": "https://queue.fal.run/fal-ai/flux-pro/v1.1-ultra/requests/req-rebind-1/response",
        })
        .to_string();
        let mut stream = accept_next(&listener, Duration::from_secs(10));
        let _ = read_request(&mut stream);
        reply(&mut stream, 200, body.as_bytes());
    });
    let submitter = FalExecutor::new(
        FalConfig::loopback_fixture(address).expect("config"),
        "fixture-key",
        bounds(),
        Duration::from_secs(5),
    )
    .expect("executor");
    let checkpoint = submitter
        .submit_generation(
            &edit("a room", ImageBackground::Opaque, ImageFormat::Jpeg),
            Duration::from_secs(5),
        )
        .await
        .expect("checkpoint");
    join_bounded(server, Duration::from_secs(10));
    let other_origin = FalExecutor::new(
        FalConfig::loopback_fixture("127.0.0.1:9".parse().expect("loopback")).expect("config"),
        "fixture-key",
        bounds(),
        Duration::from_secs(5),
    )
    .expect("executor");
    assert_eq!(
        other_origin
            .resume_generation(&checkpoint, Duration::from_secs(2))
            .await
            .map(|_| ()),
        Err(ApiError::InvalidRequest)
    );
    let other_model = FalExecutor::new(
        FalConfig::loopback_fixture(address)
            .expect("config")
            .with_models(
                "fal-ai/other-gen/v9",
                "fal-ai/flux-pro/kontext/max/multi",
                "fal-ai/birefnet/v2",
            )
            .expect("models"),
        "fixture-key",
        bounds(),
        Duration::from_secs(5),
    )
    .expect("executor");
    assert_eq!(
        other_model
            .resume_generation(&checkpoint, Duration::from_secs(2))
            .await
            .map(|_| ()),
        Err(ApiError::InvalidRequest)
    );
}

#[tokio::test]
async fn invalid_fal_polling_routes_are_rejected_at_submit_decode_and_resume() {
    use super::fal::{FLUX_ULTRA, FalAlphaCheckpoint, FalGenerationCheckpoint};
    let valid = "fal-ai/flux-pro/v1.1-ultra";
    let oversized = format!("fal-ai/{}", "a".repeat(8200));
    for (status_route, result_route) in [
        ("fal-ai/birefnet/v2", "fal-ai/birefnet/v2"),
        (valid, "fal-ai/birefnet/v2"),
        (
            "fal-ai/flux-pro/kontext/max/multi",
            "fal-ai/flux-pro/kontext/max/multi",
        ),
        (valid, "fal-ai/flux-pro"),
        ("fal-ai/flux-pro%2fv1.1-ultra", valid),
        ("fal-ai/flux-pro%7c", valid),
        ("fal-ai/flux-pro\u{7}", valid),
        ("fal-ai/flux-pro/./v1.1-ultra", valid),
        (oversized.as_str(), oversized.as_str()),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        listener.set_nonblocking(true).expect("nonblocking");
        let address = listener.local_addr().expect("address");
        let origin = format!("http://{address}");
        let status_path = format!("/{status_route}/requests/x/status");
        let result_path = format!("/{result_route}/requests/x/response");
        let checkpoint = FalGenerationCheckpoint {
            origin: origin.clone(),
            endpoint: FLUX_ULTRA.to_owned(),
            request_id: "x".to_owned(),
            status_path: status_path.clone(),
            result_path: result_path.clone(),
        };
        assert_eq!(
            FalGenerationCheckpoint::decode(&checkpoint.encode()),
            Err(ApiError::InvalidRequest)
        );
        let executor = FalExecutor::new(
            FalConfig::loopback_fixture(address).expect("config"),
            "fixture-key",
            bounds(),
            Duration::from_secs(2),
        )
        .expect("executor");
        assert_eq!(
            executor
                .resume_generation(&checkpoint, Duration::from_millis(100))
                .await
                .map(|_| ()),
            Err(ApiError::InvalidRequest)
        );
        let alpha = FalAlphaCheckpoint {
            origin: origin.clone(),
            endpoint: "fal-ai/birefnet/v2".to_owned(),
            request_id: "x".to_owned(),
            status_path: status_path.clone(),
            result_path: result_path.clone(),
        };
        // A valid alpha route is intentionally excluded from this cross-stage assertion.
        if status_route != "fal-ai/birefnet/v2" || result_route != "fal-ai/birefnet/v2" {
            assert_eq!(
                FalAlphaCheckpoint::decode(&alpha.encode()),
                Err(ApiError::InvalidRequest)
            );
            assert_eq!(
                executor
                    .resume_alpha_removal(&alpha, Duration::from_millis(100))
                    .await
                    .map(|_| ()),
                Err(ApiError::InvalidRequest)
            );
        }
        assert!(
            matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
        );
        let body = serde_json::json!({
            "request_id": "x", "status_url": format!("{origin}{status_path}"),
            "response_url": format!("{origin}{result_path}"),
        })
        .to_string();
        let server = std::thread::spawn(move || {
            let mut stream = accept_next(&listener, Duration::from_secs(3));
            let request = read_request(&mut stream);
            assert!(request.starts_with("POST /fal-ai/flux-pro/v1.1-ultra "));
            reply(&mut stream, 200, body.as_bytes());
            listener
        });
        assert_eq!(
            executor
                .submit_generation(
                    &edit("a room", ImageBackground::Opaque, ImageFormat::Jpeg),
                    Duration::from_secs(2),
                )
                .await
                .map(|_| ()),
            Err(ApiError::Output),
            "{status_route:?}, {result_route:?}"
        );
        let listener = server.join().expect("server");
        assert!(
            matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
        );
    }
}

#[tokio::test]
async fn fal_normalized_queue_routes_roundtrip_and_resume_without_resubmit() {
    use super::fal::FalGenerationCheckpoint;
    for route in ["fal-ai/flux-pro/v1.1-ultra", "fal-ai/flux-pro"] {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        listener.set_nonblocking(true).expect("nonblocking");
        let address = listener.local_addr().expect("address");
        let body = serde_json::json!({
            "request_id": "normalized", "status_url": format!("https://queue.fal.run/{route}/requests/normalized/status"),
            "response_url": format!("https://queue.fal.run/{route}/requests/normalized"),
        }).to_string();
        let status_line = format!("GET /{route}/requests/normalized/status ");
        let server = std::thread::spawn(move || {
            let mut stream = accept_next(&listener, Duration::from_secs(3));
            assert!(read_request(&mut stream).starts_with("POST /fal-ai/flux-pro/v1.1-ultra "));
            reply(&mut stream, 200, body.as_bytes());
            let mut stream = accept_next(&listener, Duration::from_secs(3));
            assert!(read_request(&mut stream).starts_with(&status_line));
            reply(&mut stream, 200, br#"{"status":"FAILED"}"#);
            listener
        });
        let make_executor = || {
            FalExecutor::new(
                FalConfig::loopback_fixture(address).expect("config"),
                "fixture-key",
                bounds(),
                Duration::from_secs(2),
            )
            .expect("executor")
        };
        let checkpoint = make_executor()
            .submit_generation(
                &edit("a room", ImageBackground::Opaque, ImageFormat::Jpeg),
                Duration::from_secs(2),
            )
            .await
            .expect("checkpoint");
        let restored = FalGenerationCheckpoint::decode(&checkpoint.encode()).expect("roundtrip");
        assert_eq!(restored, checkpoint);
        assert_eq!(
            make_executor()
                .resume_generation(&restored, Duration::from_secs(2))
                .await
                .map(|_| ()),
            Err(ApiError::Provider(500))
        );
        let listener = server.join().expect("server");
        assert!(
            matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
        );
    }
}

#[tokio::test]
async fn maximum_fal_model_and_supplier_id_roundtrip_after_submit() {
    use super::fal::{FalAlphaCheckpoint, FalGenerationCheckpoint};
    let endpoint = format!("owner/{}", "a".repeat(250));
    let request_id = "x".repeat(256);
    let status_path = format!("/{endpoint}/requests/{request_id}/status");
    let result_path = format!("/{endpoint}/requests/{request_id}/response");
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    listener.set_nonblocking(true).expect("nonblocking");
    let address = listener.local_addr().expect("address");
    let body = serde_json::json!({
        "request_id": request_id, "status_url": format!("https://queue.fal.run{status_path}"),
        "response_url": format!("https://queue.fal.run{result_path}"),
    })
    .to_string();
    let server = std::thread::spawn(move || {
        let mut stream = accept_next(&listener, Duration::from_secs(3));
        assert!(read_request(&mut stream).starts_with("POST "));
        reply(&mut stream, 200, body.as_bytes());
    });
    let executor = FalExecutor::new(
        FalConfig::loopback_fixture(address)
            .expect("config")
            .with_models(
                &endpoint,
                "fal-ai/flux-pro/kontext/max/multi",
                "fal-ai/birefnet/v2",
            )
            .expect("models"),
        "fixture-key",
        bounds(),
        Duration::from_secs(2),
    )
    .expect("executor");
    let mut checkpoint = executor
        .submit_generation(
            &edit("a room", ImageBackground::Opaque, ImageFormat::Jpeg),
            Duration::from_secs(2),
        )
        .await
        .expect("maximum checkpoint");
    assert_eq!(
        FalGenerationCheckpoint::decode(&checkpoint.encode()).expect("roundtrip"),
        checkpoint
    );
    join_bounded(server, Duration::from_secs(3));
    let origin_prefix = "https://queue.example.test/";
    checkpoint.origin = format!("{origin_prefix}{}", "b".repeat(8192 - origin_prefix.len()));
    assert_eq!(checkpoint.origin.len(), 8192);
    assert_eq!(
        FalGenerationCheckpoint::decode(&checkpoint.encode()).expect("maximum origin"),
        checkpoint
    );
    let alpha = FalAlphaCheckpoint {
        origin: checkpoint.origin.clone(),
        endpoint,
        request_id,
        status_path,
        result_path,
    };
    assert_eq!(
        FalAlphaCheckpoint::decode(&alpha.encode()).expect("alpha roundtrip"),
        alpha
    );
    checkpoint.origin.push('b');
    assert_eq!(
        FalGenerationCheckpoint::decode(&checkpoint.encode()),
        Err(ApiError::InvalidRequest)
    );
    let too_large = format!("fal-generation-v2|{}", "a".repeat(24576));
    assert_eq!(
        FalGenerationCheckpoint::decode(&too_large),
        Err(ApiError::InvalidRequest)
    );
    assert_eq!(
        FalAlphaCheckpoint::decode(&too_large),
        Err(ApiError::InvalidRequest)
    );
}

#[tokio::test]
async fn observed_queue_decline_is_closed_and_accepted_poll_never_reauthorizes_submit() {
    use super::{DeclineContract, SubmissionEvidence};
    let body = br#"{"error":"queue limit","request_id":null}"#;
    for accepted in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        listener.set_nonblocking(true).expect("bounded accept");
        let address = listener.local_addr().expect("address");
        let server = std::thread::spawn(move || {
            let mut stream = accept_next(&listener, Duration::from_secs(5));
            assert!(read_request(&mut stream).starts_with("POST "));
            if accepted {
                reply(&mut stream,200,br#"{"request_id":"req-1","status_url":"https://queue.fal.run/fal-ai/flux-pro/v1.1-ultra/requests/req-1/status","response_url":"https://queue.fal.run/fal-ai/flux-pro/v1.1-ultra/requests/req-1/response"}"#);
                let mut stream = accept_next(&listener, Duration::from_secs(5));
                assert!(read_request(&mut stream).starts_with("GET "));
                reply(&mut stream, 429, body);
            } else {
                reply(&mut stream, 429, body);
            }
        });
        let executor = FalExecutor::new(
            FalConfig::loopback_fixture(address).expect("config"),
            "fixture-key",
            bounds(),
            Duration::from_secs(5),
        )
        .expect("executor");
        let submit = executor
            .submit_generation_observed(
                &edit("a room", ImageBackground::Opaque, ImageFormat::Jpeg),
                Duration::from_secs(5),
            )
            .await;
        let failure = if accepted {
            executor
                .resume_generation_observed(
                    &submit.expect("accepted checkpoint"),
                    Duration::from_secs(5),
                )
                .await
                .expect_err("poll 429")
        } else {
            submit.expect_err("submit 429")
        };
        assert_eq!(failure.error, ApiError::RateLimited);
        assert_eq!(
            failure.submission,
            if accepted {
                SubmissionEvidence::AcceptedJob
            } else {
                SubmissionEvidence::RejectedSubmit(DeclineContract::FalQueue429V1)
            }
        );
        assert_eq!(
            failure.retryable_submit_decline(1_700_000_000_000),
            if accepted {
                None
            } else {
                Some(DeclineContract::FalQueue429V1)
            }
        );
        assert_eq!(failure.response.expect("exact response").body, body);
        server.join().expect("fixture");
    }
}
