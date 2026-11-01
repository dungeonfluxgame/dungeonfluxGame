//! Failure-focused loopback tests for the private BFL backend.
//!
//! No live calls. Moderation stays terminal, delivery stays credential-free,
//! and submit/poll/download boundaries are diagnosed separately.

use super::bfl::{BflConfig, BflExecutor, BflPortraitCheckpoint};
use super::{ApiError, ImageBackground, ImageEdit, ImageFormat, ImageModeration, ImageQuality};
use super::{ImageEditModel, OperationalBounds};
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::Arc,
    sync::atomic::{AtomicUsize, Ordering},
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
        0xff,
        0xd8,
        0xff,
        0xc0,
        0x00,
        0x08,
        0x08,
        (height >> 8) as u8,
        height as u8,
        (width >> 8) as u8,
        width as u8,
        0x01,
        0xff,
        0xd9,
    ]
}

fn read_request(stream: &mut std::net::TcpStream) -> String {
    stream
        .set_nonblocking(false)
        .expect("blocking fixture stream");
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

fn reply_bytes(stream: &mut std::net::TcpStream, status: u16, mime: &str, body: &[u8]) {
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
        executor
            .edit(&bad, Duration::from_secs(5))
            .await
            .map(|_| ()),
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
            .edit(
                &portrait("fictional adult adventurer"),
                Duration::from_secs(5)
            )
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
            .edit(
                &portrait("fictional adult adventurer"),
                Duration::from_secs(5)
            )
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
            .edit_diagnosed(
                &portrait("fictional adult adventurer"),
                Duration::from_secs(5),
            )
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
        .edit(
            &portrait("fictional adult adventurer"),
            Duration::from_secs(5),
        )
        .await
        .expect("image");
    assert_eq!((success.image.width, success.image.height), (1024, 1536));
    assert_eq!(success.image.bytes, minimal_jpeg(1024, 1536));
    assert_eq!(success.credits.as_deref(), Some("7"));
    server.join().expect("server");
}

#[tokio::test]
async fn submit_persist_recreate_resume_makes_one_post() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let posts = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&posts);
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("submit");
        let request = read_request(&mut stream);
        assert!(request.starts_with("POST /v1/flux-3-image "));
        counted.fetch_add(1, Ordering::SeqCst);
        let body = serde_json::json!({
            "polling_url": format!("http://{address}/v1/get_result?id=job-resume"),
            "cost": 7,
        })
        .to_string();
        reply_bytes(&mut stream, 200, "application/json", body.as_bytes());
        let (mut stream, _) = listener.accept().expect("poll");
        let request = read_request(&mut stream);
        assert!(request.starts_with("GET /v1/get_result"));
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
        let request = read_request(&mut stream);
        assert!(request.starts_with("GET /image.jpg "));
        let lowered = request.to_ascii_lowercase();
        assert!(!lowered.contains("fixture-key"));
        assert!(!lowered.contains("x-key:"));
        assert!(!lowered.contains("authorization:"));
        reply_bytes(&mut stream, 200, "image/jpeg", &minimal_jpeg(1024, 1536));
    });
    let submitter = BflExecutor::new(
        BflConfig::loopback_fixture(address).expect("config"),
        "fixture-key",
        bounds(),
        Duration::from_secs(5),
    )
    .expect("executor");
    let checkpoint = submitter
        .submit_portrait(
            &portrait("fictional adult adventurer"),
            Duration::from_secs(5),
        )
        .await
        .expect("checkpoint");
    assert_eq!(checkpoint.request_id, "job-resume");
    assert_eq!(checkpoint.credits.as_deref(), Some("7"));
    let persisted = checkpoint.encode();
    let restored = BflPortraitCheckpoint::decode(&persisted).expect("decode");
    assert_eq!(restored, checkpoint);
    let debug = format!("{restored:?}");
    assert!(!debug.contains("job-resume"));
    assert!(!debug.contains("get_result"));
    drop(submitter);
    let resumer = BflExecutor::new(
        BflConfig::loopback_fixture(address).expect("config"),
        "fixture-key",
        bounds(),
        Duration::from_secs(5),
    )
    .expect("executor");
    let success = resumer
        .resume_portrait(&restored, Duration::from_secs(5))
        .await
        .expect("resume");
    assert_eq!((success.image.width, success.image.height), (1024, 1536));
    assert_eq!(success.image.bytes, minimal_jpeg(1024, 1536));
    assert_eq!(success.credits.as_deref(), Some("7"));
    assert_eq!(posts.load(Ordering::SeqCst), 1);
    server.join().expect("server");
}

#[tokio::test]
async fn a_forged_polling_locator_is_rejected_before_any_egress() {
    let executor = BflExecutor::new(
        BflConfig::loopback_fixture("127.0.0.1:9".parse().expect("loopback")).expect("config"),
        "fixture-key",
        bounds(),
        Duration::from_secs(5),
    )
    .expect("executor");
    for forged in [
        "bfl-portrait-v2|http://127.0.0.1:9|flux-3-image|https://evil.test/v1/get_result?id=x#frag|x|",
        "bfl-portrait-v2|http://127.0.0.1:9|flux-3-image|https://user@api.bfl.ai/v1/get_result?id=x|x|",
        "bfl-portrait-v2|http://127.0.0.1:9|flux-3-image|https://api.bfl.ai/other?id=x|x|",
        "bfl-portrait-v2|http://127.0.0.1:9|flux-3-image|https://evil.test/v1/get_result?id=x|y|",
        "bfl-portrait-v1|https://evil.test/v1/get_result?id=x|x|",
        "not-a-checkpoint",
    ] {
        match BflPortraitCheckpoint::decode(forged) {
            Err(ApiError::InvalidRequest) => {}
            Ok(checkpoint) => assert_eq!(
                executor
                    .resume_portrait(&checkpoint, Duration::from_secs(2))
                    .await
                    .map(|_| ()),
                Err(ApiError::InvalidRequest)
            ),
            Err(other) => panic!("wrong refusal for {forged:?}: {other:?}"),
        }
    }
    let hostile = BflPortraitCheckpoint::decode(
        "bfl-portrait-v2|http://127.0.0.1:9|flux-3-image|https://evil.test/v1/get_result?id=x|x|",
    )
    .expect("hostile locator keeps its shape at decode");
    assert_eq!(
        executor
            .resume_portrait(&hostile, Duration::from_secs(2))
            .await
            .map(|_| ()),
        Err(ApiError::InvalidRequest)
    );
}

#[tokio::test]
async fn a_deadline_during_resume_never_resubmits() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let posts = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&posts);
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("submit");
        let request = read_request(&mut stream);
        assert!(request.starts_with("POST "));
        counted.fetch_add(1, Ordering::SeqCst);
        let body = serde_json::json!({
            "polling_url": format!("http://{address}/v1/get_result?id=job-slow"),
        })
        .to_string();
        reply_bytes(&mut stream, 200, "application/json", body.as_bytes());
        let (mut stream, _) = listener.accept().expect("poll");
        let _ = read_request(&mut stream);
        reply_bytes(
            &mut stream,
            200,
            "application/json",
            b"{\"status\":\"Pending\"}",
        );
    });
    let submitter = BflExecutor::new(
        BflConfig::loopback_fixture(address).expect("config"),
        "fixture-key",
        bounds(),
        Duration::from_secs(5),
    )
    .expect("executor");
    let checkpoint = submitter
        .submit_portrait(
            &portrait("fictional adult adventurer"),
            Duration::from_secs(5),
        )
        .await
        .expect("checkpoint");
    drop(submitter);
    let resumer = BflExecutor::new(
        BflConfig::loopback_fixture(address).expect("config"),
        "fixture-key",
        bounds(),
        Duration::from_secs(5),
    )
    .expect("executor");
    assert_eq!(
        resumer
            .resume_portrait(&checkpoint, Duration::from_millis(60))
            .await
            .map(|_| ()),
        Err(ApiError::Transport(
            df_providers::NativeHttpRefusal::Deadline
        ))
    );
    assert_eq!(posts.load(Ordering::SeqCst), 1);
    server.join().expect("server");
}

#[tokio::test]
async fn a_moderated_terminal_status_during_resume_never_resubmits() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let posts = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&posts);
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("submit");
        let request = read_request(&mut stream);
        assert!(request.starts_with("POST "));
        counted.fetch_add(1, Ordering::SeqCst);
        let body = serde_json::json!({
            "polling_url": format!("http://{address}/v1/get_result?id=job-mod"),
        })
        .to_string();
        reply_bytes(&mut stream, 200, "application/json", body.as_bytes());
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().expect("poll");
            let request = read_request(&mut stream);
            assert!(request.starts_with("GET /v1/get_result"));
            reply_bytes(
                &mut stream,
                503,
                "application/json",
                b"{\"status\":\"Content Moderated\"}",
            );
        }
    });
    let submitter = BflExecutor::new(
        BflConfig::loopback_fixture(address).expect("config"),
        "fixture-key",
        bounds(),
        Duration::from_secs(5),
    )
    .expect("executor");
    let checkpoint = submitter
        .submit_portrait(
            &portrait("fictional adult adventurer"),
            Duration::from_secs(5),
        )
        .await
        .expect("checkpoint");
    drop(submitter);
    let resumer = BflExecutor::new(
        BflConfig::loopback_fixture(address).expect("config"),
        "fixture-key",
        bounds(),
        Duration::from_secs(5),
    )
    .expect("executor");
    assert_eq!(
        resumer
            .resume_portrait(&checkpoint, Duration::from_secs(5))
            .await
            .map(|_| ()),
        Err(ApiError::Refused)
    );
    let failure = resumer
        .resume_portrait_diagnosed(&checkpoint, Duration::from_secs(5))
        .await
        .expect_err("diagnosed refusal");
    assert_eq!(failure.stage, "poll");
    assert_eq!(failure.observation.error, ApiError::Refused);
    assert_eq!(posts.load(Ordering::SeqCst), 1);
    server.join().expect("server");
}

#[tokio::test]
async fn a_custom_model_route_flows_through_the_submit_path() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("submit");
        let request = read_request(&mut stream);
        assert!(request.starts_with("POST /v1/flux-9-custom "));
        let body = serde_json::json!({
            "polling_url": format!("http://{address}/v1/get_result?id=job-custom"),
        })
        .to_string();
        reply_bytes(&mut stream, 200, "application/json", body.as_bytes());
    });
    let executor = BflExecutor::new(
        BflConfig::loopback_fixture(address)
            .expect("config")
            .with_route("flux-9-custom")
            .expect("route"),
        "fixture-key",
        bounds(),
        Duration::from_secs(5),
    )
    .expect("executor");
    let checkpoint = executor
        .submit_portrait(
            &portrait("fictional adult adventurer"),
            Duration::from_secs(5),
        )
        .await
        .expect("checkpoint");
    assert_eq!(checkpoint.request_id, "job-custom");
    server.join().expect("server");
}

#[test]
fn operator_origins_and_routes_are_validated() {
    assert!(BflConfig::operator("https://api.bfl.ai").is_ok());
    assert!(BflConfig::operator("https://api.bfl.ai:8443").is_err());
    assert_eq!(
        BflConfig::operator("http://127.example.test").map(|_| ()),
        Err(ApiError::InvalidRequest)
    );
    assert_eq!(
        BflConfig::operator("https://api.example.test/%2e").map(|_| ()),
        Err(ApiError::InvalidRequest)
    );
    assert_eq!(
        BflConfig::operator("https://user@example.test").map(|_| ()),
        Err(ApiError::InvalidRequest)
    );
    assert_eq!(
        BflConfig::operator("https://api.example.test/?q=1").map(|_| ()),
        Err(ApiError::InvalidRequest)
    );
    let address: std::net::SocketAddr = "127.0.0.1:9".parse().expect("loopback");
    assert_eq!(
        BflConfig::loopback_fixture(address)
            .expect("config")
            .with_route("bad route?")
            .map(|_| ()),
        Err(ApiError::InvalidRequest)
    );
    assert_eq!(
        BflConfig::loopback_fixture(address)
            .expect("config")
            .with_route("a/b")
            .map(|_| ()),
        Err(ApiError::InvalidRequest)
    );
    let debug = format!(
        "{:?}",
        BflConfig::operator("https://api.bfl.ai").expect("config")
    );
    assert!(!debug.contains("api.bfl.ai"));
}

#[tokio::test]
async fn a_missing_polling_id_is_an_output_failure_never_pending() {
    for body in [
        serde_json::json!({"polling_url": "http://127.0.0.1:9/v1/get_result"}).to_string(),
        serde_json::json!({"polling_url": "http://127.0.0.1:9/v1/get_result?id="}).to_string(),
        serde_json::json!({"polling_url": "http://127.0.0.1:9/v1/get_result?xid=x"}).to_string(),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        let address = listener.local_addr().expect("address");
        let owned = body.clone();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("submit");
            let _ = read_request(&mut stream);
            let located = owned.replace("127.0.0.1:9", &address.to_string());
            reply_bytes(&mut stream, 200, "application/json", located.as_bytes());
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
                .submit_portrait(
                    &portrait("fictional adult adventurer"),
                    Duration::from_secs(5)
                )
                .await
                .map(|_| ()),
            Err(ApiError::Output),
            "{body:?}"
        );
        server.join().expect("server");
    }
}

#[tokio::test]
async fn a_wrong_typed_or_oversized_cost_is_an_output_failure() {
    for body in [
        serde_json::json!({"polling_url": "http://127.0.0.1:9/v1/get_result?id=x", "cost": "7"})
            .to_string(),
        serde_json::json!({"polling_url": "http://127.0.0.1:9/v1/get_result?id=x", "cost": -3})
            .to_string(),
        serde_json::json!({"polling_url": "http://127.0.0.1:9/v1/get_result?id=x", "cost": true})
            .to_string(),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        let address = listener.local_addr().expect("address");
        let owned = body.clone();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("submit");
            let _ = read_request(&mut stream);
            let located = owned.replace("127.0.0.1:9", &address.to_string());
            reply_bytes(&mut stream, 200, "application/json", located.as_bytes());
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
                .submit_portrait(
                    &portrait("fictional adult adventurer"),
                    Duration::from_secs(5)
                )
                .await
                .map(|_| ()),
            Err(ApiError::Output),
            "{body:?}"
        );
        server.join().expect("server");
    }
}

#[test]
fn present_credit_receipts_are_bounded_before_allocation() {
    let origin = "http://127.0.0.1:9";
    let good = format!(
        "bfl-portrait-v2|{origin}|flux-3-image|{origin}/v1/get_result?id=private-request|private-request|7"
    );
    assert!(BflPortraitCheckpoint::decode(&good).is_ok());
    let absent = format!("bfl-portrait-v2|{origin}|flux-3-image|{origin}/v1/get_result?id=x|x|");
    assert!(
        BflPortraitCheckpoint::decode(&absent)
            .expect("decode")
            .credits
            .is_none()
    );
    for bad_credits in ["abc", "-3", "7e3", &"7".repeat(33), ".5", "5."] {
        let forged = format!(
            "bfl-portrait-v2|{origin}|flux-3-image|{origin}/v1/get_result?id=x|x|{bad_credits}"
        );
        assert_eq!(
            BflPortraitCheckpoint::decode(&forged),
            Err(ApiError::InvalidRequest),
            "{bad_credits:?}"
        );
    }
    let checkpoint = BflPortraitCheckpoint::decode(&good).expect("decode");
    let debug = format!("{checkpoint:?}");
    assert!(!debug.contains("get_result"));
    assert!(!debug.contains(origin));
    assert!(!debug.contains("private-request"));
}

#[tokio::test]
async fn a_custom_operator_never_falls_back_to_live_bfl_hosts() {
    let custom = BflExecutor::new(
        BflConfig::operator("http://127.0.0.1:18080").expect("config"),
        "custom-key",
        bounds(),
        Duration::from_secs(5),
    )
    .expect("executor");
    for locator in [
        "https://api.bfl.ai/v1/get_result?id=x",
        "https://api.regional.bfl.ai/v1/get_result?id=x",
    ] {
        let checkpoint = BflPortraitCheckpoint {
            origin: "http://127.0.0.1:18080".to_owned(),
            model_route: super::bfl::BFL_FLUX3_IMAGE.to_owned(),
            polling_url: locator.to_owned(),
            request_id: "x".to_owned(),
            credits: None,
        };
        assert_eq!(
            custom
                .resume_portrait(&checkpoint, Duration::from_secs(2))
                .await
                .map(|_| ()),
            Err(ApiError::InvalidRequest),
            "{locator:?}"
        );
    }
}

#[tokio::test]
async fn a_bfl_checkpoint_rebound_to_another_origin_route_or_id_is_refused() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("submit");
        let _ = read_request(&mut stream);
        let body = serde_json::json!({
            "polling_url": format!("http://{address}/v1/get_result?id=job-bind"),
            "cost": 7,
        })
        .to_string();
        reply_bytes(&mut stream, 200, "application/json", body.as_bytes());
    });
    let submitter = BflExecutor::new(
        BflConfig::loopback_fixture(address).expect("config"),
        "fixture-key",
        bounds(),
        Duration::from_secs(5),
    )
    .expect("executor");
    let checkpoint = submitter
        .submit_portrait(
            &portrait("fictional adult adventurer"),
            Duration::from_secs(5),
        )
        .await
        .expect("checkpoint");
    server.join().expect("server");
    assert_eq!(checkpoint.request_id, "job-bind");
    let other_origin = BflExecutor::new(
        BflConfig::loopback_fixture("127.0.0.1:9".parse().expect("loopback")).expect("config"),
        "fixture-key",
        bounds(),
        Duration::from_secs(5),
    )
    .expect("executor");
    assert_eq!(
        other_origin
            .resume_portrait(&checkpoint, Duration::from_secs(2))
            .await
            .map(|_| ()),
        Err(ApiError::InvalidRequest)
    );
    let other_route = BflExecutor::new(
        BflConfig::loopback_fixture(address)
            .expect("config")
            .with_route("flux-9-custom")
            .expect("route"),
        "fixture-key",
        bounds(),
        Duration::from_secs(5),
    )
    .expect("executor");
    assert_eq!(
        other_route
            .resume_portrait(&checkpoint, Duration::from_secs(2))
            .await
            .map(|_| ()),
        Err(ApiError::InvalidRequest)
    );
    let mut mismatched = checkpoint.clone();
    mismatched.request_id = "other".to_owned();
    let local = BflExecutor::new(
        BflConfig::loopback_fixture(address).expect("config"),
        "fixture-key",
        bounds(),
        Duration::from_secs(5),
    )
    .expect("executor");
    assert_eq!(
        local
            .resume_portrait(&mismatched, Duration::from_secs(2))
            .await
            .map(|_| ()),
        Err(ApiError::InvalidRequest)
    );
}

#[tokio::test]
async fn every_duplicate_bfl_id_is_rejected_at_decode_resume_and_submit() {
    for query in [
        "id=first&id=second",
        "id=first&id=first",
        "id=first&id=",
        "id=first&id=bad!",
        "id=first&other=ok&id=second",
        "id=first&malformed",
        "id=&id=first",
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        listener.set_nonblocking(true).expect("nonblocking");
        let address = listener.local_addr().expect("address");
        let origin = format!("http://{address}");
        let polling_url = format!("{origin}/v1/get_result?{query}");
        let checkpoint = BflPortraitCheckpoint {
            origin,
            model_route: "flux-3-image".to_owned(),
            polling_url: polling_url.clone(),
            request_id: "first".to_owned(),
            credits: None,
        };
        assert_eq!(
            BflPortraitCheckpoint::decode(&checkpoint.encode()),
            Err(ApiError::InvalidRequest),
            "{query}"
        );
        let executor = BflExecutor::new(
            BflConfig::loopback_fixture(address).expect("config"),
            "fixture-key",
            bounds(),
            Duration::from_secs(2),
        )
        .expect("executor");
        assert_eq!(
            executor
                .resume_portrait(&checkpoint, Duration::from_millis(100))
                .await
                .map(|_| ()),
            Err(ApiError::InvalidRequest)
        );
        assert!(
            matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
        );
        let body = serde_json::json!({"polling_url": polling_url}).to_string();
        let server = std::thread::spawn(move || {
            let until = std::time::Instant::now() + Duration::from_secs(3);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(std::time::Instant::now() < until, "submit timeout");
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("{error}"),
                }
            };
            assert!(read_request(&mut stream).starts_with("POST /v1/flux-3-image "));
            reply_bytes(&mut stream, 200, "application/json", body.as_bytes());
            listener
        });
        assert_eq!(
            executor
                .submit_portrait(
                    &portrait("fictional adult adventurer"),
                    Duration::from_secs(2)
                )
                .await
                .map(|_| ()),
            Err(ApiError::Output),
            "{query}"
        );
        let listener = server.join().expect("server");
        assert!(
            matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
        );
    }
}

#[tokio::test]
async fn custom_bfl_base_paths_preserve_escapes_through_submit_and_resume() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    listener.set_nonblocking(true).expect("nonblocking");
    let address = listener.local_addr().expect("address");
    let origin = format!("http://{address}/team%20one/~operator");
    let polling_url = format!("{origin}/v1/get_result?id=base-job");
    let body = serde_json::json!({"polling_url": polling_url, "cost": 7}).to_string();
    let server = std::thread::spawn(move || {
        for (expected, response) in [
            (
                "POST /team%20one/~operator/v1/flux-3-image ",
                body.as_bytes(),
            ),
            (
                "GET /team%20one/~operator/v1/get_result?id=base-job ",
                br#"{"status":"Content Moderated"}"#,
            ),
        ] {
            let until = std::time::Instant::now() + Duration::from_secs(3);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(std::time::Instant::now() < until, "request not observed");
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("fixture accept: {error}"),
                }
            };
            assert!(read_request(&mut stream).starts_with(expected));
            reply_bytes(&mut stream, 200, "application/json", response);
        }
        listener
    });
    let make_executor = || {
        BflExecutor::new(
            BflConfig::operator(&origin).expect("config"),
            "fixture-key",
            bounds(),
            Duration::from_secs(2),
        )
        .expect("executor")
    };
    let checkpoint = make_executor()
        .submit_portrait(
            &portrait("fictional adult adventurer"),
            Duration::from_secs(2),
        )
        .await
        .expect("checkpoint");
    let restored = BflPortraitCheckpoint::decode(&checkpoint.encode()).expect("roundtrip");
    assert_eq!(restored, checkpoint);
    assert_eq!(
        make_executor()
            .resume_portrait_diagnosed(&restored, Duration::from_secs(2))
            .await
            .map(|_| ())
            .map_err(|failure| failure.observation.error),
        Err(ApiError::Refused)
    );
    let listener = server.join().expect("server");
    assert!(
        matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
    );
}

#[tokio::test]
async fn mixed_case_operator_origin_accepts_canonical_locator_after_recreation() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    listener.set_nonblocking(true).expect("nonblocking");
    let address = listener.local_addr().expect("address");
    let origin = format!("HTTP://LOCALHOST:{}/team%20one", address.port());
    let polling_url = format!(
        "http://localhost:{}/team%20one/v1/get_result?id=case-job",
        address.port()
    );
    let body = serde_json::json!({"polling_url":polling_url}).to_string();
    let server = std::thread::spawn(move || {
        for (expected, response) in [
            ("POST /team%20one/v1/flux-3-image ", body.as_bytes()),
            (
                "GET /team%20one/v1/get_result?id=case-job ",
                br#"{"status":"Content Moderated"}"#,
            ),
        ] {
            let until = std::time::Instant::now() + Duration::from_secs(4);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(std::time::Instant::now() < until, "request not observed");
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("fixture accept: {error}"),
                }
            };
            assert!(read_request(&mut stream).starts_with(expected));
            reply_bytes(&mut stream, 200, "application/json", response);
        }
        listener
    });
    let make_executor = || {
        BflExecutor::new(
            BflConfig::operator(&origin).expect("config"),
            "fixture-key",
            bounds(),
            Duration::from_secs(3),
        )
        .expect("executor")
    };
    let checkpoint = make_executor()
        .submit_portrait(
            &portrait("synthetic adult adventurer"),
            Duration::from_secs(3),
        )
        .await
        .expect("receipt");
    let mut encoded = checkpoint.encode();
    encoded = encoded.replacen(&checkpoint.origin, &origin, 1);
    let restored = BflPortraitCheckpoint::decode(&encoded).expect("old spelling normalized");
    assert_eq!(restored.origin, checkpoint.origin);
    assert_eq!(
        make_executor()
            .resume_portrait_diagnosed(&restored, Duration::from_secs(3))
            .await
            .map(|_| ())
            .map_err(|failure| failure.observation.error),
        Err(ApiError::Refused)
    );
    let listener = server.join().expect("server");
    assert!(
        matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
    );
}

#[tokio::test]
async fn fragment_after_an_extra_query_pair_is_invalid_without_credential_egress() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    listener.set_nonblocking(true).expect("nonblocking");
    let address = listener.local_addr().expect("address");
    let origin = format!("http://{address}");
    let checkpoint = BflPortraitCheckpoint {
        origin: origin.clone(),
        model_route: "flux-3-image".to_owned(),
        polling_url: format!("{origin}/v1/get_result?id=x&extra=y#fragment"),
        request_id: "x".to_owned(),
        credits: None,
    };
    assert_eq!(
        BflPortraitCheckpoint::decode(&checkpoint.encode()),
        Err(ApiError::InvalidRequest)
    );
    let executor = BflExecutor::new(
        BflConfig::loopback_fixture(address).expect("config"),
        "synthetic-key",
        bounds(),
        Duration::from_secs(1),
    )
    .expect("executor");
    assert_eq!(
        executor
            .resume_portrait_diagnosed(&checkpoint, Duration::from_millis(50))
            .await
            .map(|_| ())
            .map_err(|failure| failure.observation.error),
        Err(ApiError::InvalidRequest)
    );
    assert!(matches!(listener.accept(),Err(error) if error.kind()==std::io::ErrorKind::WouldBlock));
}

#[tokio::test]
async fn undocumented_bfl_429_keeps_exact_response_without_retry_authority() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let body = br#"{"error":{"code":"rate_limit_exceeded"}}"#;
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let _ = read_request(&mut stream);
        reply_bytes(&mut stream, 429, "application/json", body);
    });
    let executor = BflExecutor::new(
        BflConfig::loopback_fixture(address).expect("config"),
        "fixture-key",
        bounds(),
        Duration::from_secs(5),
    )
    .expect("executor");
    let failure = executor
        .submit_portrait_observed(
            &portrait("fictional adult adventurer"),
            Duration::from_secs(5),
        )
        .await
        .expect_err("429");
    assert_eq!(failure.error, ApiError::RateLimited);
    assert_eq!(failure.submission, super::SubmissionEvidence::Unknown);
    assert_eq!(failure.retryable_submit_decline(), None);
    assert_eq!(failure.response.expect("exact response").body, body);
    server.join().expect("fixture");
}
