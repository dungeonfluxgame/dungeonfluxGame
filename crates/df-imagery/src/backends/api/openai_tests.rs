//! Failure-focused loopback tests for the private OpenAI backend.
//!
//! No live calls. Each fixture speaks minimal HTTP on loopback and asserts
//! the single-call contract: no retry, reword or reroute.

use super::openai::{OpenAiConfig, OpenAiExecutor};
use super::{ApiError, ImageBackground, ImageEdit, ImageFormat, ImageModeration, ImageQuality};
use super::{ImageEditModel, ImageUsage, OperationalBounds};
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
    String::from_utf8(read_request_bytes(stream)).expect("request text")
}

fn read_request_bytes(stream: &mut std::net::TcpStream) -> Vec<u8> {
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
                return bytes;
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
        reply(
            &mut stream,
            400,
            b"{\"error\":{\"code\":\"moderation_blocked\"}}",
        );
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
                Duration::from_secs(5)
            )
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
        (
            "{\"error\":{\"code\":\"rate_limit\"}}",
            ApiError::RateLimited,
        ),
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
                .edit(
                    &edit("a tavern at dusk", ImageFormat::Jpeg),
                    Duration::from_secs(5)
                )
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
            .edit(
                &edit("a tavern at dusk", ImageFormat::Jpeg),
                Duration::from_secs(5)
            )
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
            .edit(
                &edit("a tavern at dusk", ImageFormat::Jpeg),
                Duration::from_secs(5)
            )
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
        .edit(
            &edit("a tavern at dusk", ImageFormat::Jpeg),
            Duration::from_secs(5),
        )
        .await
        .expect("image");
    assert_eq!(image.bytes, bytes);
    assert_eq!((image.width, image.height), (1024, 1024));
    assert_eq!(image.usage.text_input_tokens, 7);
    assert_eq!(image.usage.image_input_tokens, 3);
    assert_eq!(image.usage.output_tokens, 11);
    server.join().expect("server");
}

#[tokio::test]
async fn a_partial_usage_report_on_a_delivered_image_stays_unknown_instead_of_reported_zeros() {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    let bytes = minimal_jpeg(1024, 1024);
    let body = serde_json::json!({
        "data": [{"b64_json": STANDARD.encode(&bytes)}],
        "usage": {"input_tokens_details": {"text_tokens": 7}, "output_tokens": 11}
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
        .edit(
            &edit("a tavern at dusk", ImageFormat::Jpeg),
            Duration::from_secs(5),
        )
        .await
        .expect("image");
    assert_eq!(image.bytes, bytes);
    assert_eq!(image.usage, ImageUsage::default());
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

#[test]
fn configured_model_ids_and_operator_endpoints_are_validated() {
    assert_eq!(
        super::ImageEditModel::parse_model_id("gpt-image-2.5-flare").map(|_| ()),
        Ok(())
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
    assert_eq!(
        super::ImageEditModel::parse_model_id("operator/model:revision").map(|_| ()),
        Ok(())
    );
    assert!(
        OpenAiConfig::operator(
            "https://api.openai.com/v1/images/edits",
            "https://api.openai.com/v1/images/generations"
        )
        .is_ok()
    );
    assert_eq!(
        OpenAiConfig::operator(
            "https://user@example.test/v1/images/edits",
            "https://api.openai.com/v1/images/generations"
        )
        .map(|_| ()),
        Err(ApiError::InvalidRequest)
    );
    assert_eq!(
        OpenAiConfig::operator(
            "https://api.openai.com/v1/images/edits?x=1",
            "https://api.openai.com/v1/images/generations"
        )
        .map(|_| ()),
        Err(ApiError::InvalidRequest)
    );
    let debug = format!(
        "{:?}",
        OpenAiConfig::operator(
            "https://api.openai.com/v1/images/edits",
            "https://api.openai.com/v1/images/generations"
        )
        .expect("config")
    );
    assert!(!debug.contains("api.openai.com"));
}

#[test]
fn operator_endpoint_paths_preserve_safe_uri_escapes_and_refuse_ambiguous_routes() {
    for path in [
        "/v1/team%20one/generations",
        "/v1/%E2%98%83/generations",
        "/v1/~!$&'()*+,;=:@/generations",
        "/v1/name..part/generations",
        "/v1//generations",
        "/v1/%2eversion/generations",
        "/v1/%23%3f%40/generations",
    ] {
        let endpoint = format!("https://api.openai.com{path}");
        assert!(
            crate::config::ApiEndpoint::new(&endpoint).is_ok(),
            "config: {path}"
        );
        assert_eq!(
            super::check_endpoint_url(&endpoint),
            Ok(false),
            "backend: {path}"
        );
        assert!(OpenAiConfig::operator(&endpoint, &endpoint).is_ok());
    }
    assert_eq!(
        super::check_endpoint_url("HTTP://[::1]:1234/team%20one"),
        Ok(true)
    );
    for path in [
        "/v1/%",
        "/v1/%2",
        "/v1/%gg",
        "/v1/%00",
        "/v1/%1f",
        "/v1/%7F",
        "/v1/%2fescape",
        "/v1/%5Cescape",
        "/v1/%2e",
        "/v1/.%2e",
        "/v1/%2E%2e",
        "/v1/../escape",
        "/v1/./escape",
        "/v1/%252e%252e/escape",
    ] {
        assert_eq!(
            super::check_endpoint_url(&format!("https://api.example.test{path}")),
            Err(ApiError::InvalidRequest),
            "{path}"
        );
    }
    for endpoint in [
        "https://user:password@api.example.test/path",
        "https://api.example.test/path?query",
        "https://api.example.test/path#fragment",
    ] {
        assert_eq!(
            super::check_endpoint_url(endpoint),
            Err(ApiError::InvalidRequest)
        );
    }
}

#[tokio::test]
async fn a_configured_escaped_endpoint_reaches_the_wire_unchanged() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    listener.set_nonblocking(true).expect("nonblocking");
    let address = listener.local_addr().expect("address");
    let path = "/team%20one/~!$&'()*+,;=:@/generations";
    let endpoint = format!("http://{address}{path}");
    assert!(crate::config::ApiEndpoint::new(&endpoint).is_ok());
    let server = std::thread::spawn(move || {
        let until = std::time::Instant::now() + Duration::from_secs(3);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(std::time::Instant::now() < until, "submit not observed");
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("fixture accept: {error}"),
            }
        };
        let request = read_request(&mut stream);
        assert!(
            request.starts_with(&format!("POST {path} HTTP/1.1")),
            "{request}"
        );
        reply(&mut stream, 200, br#"{"data":[]}"#);
        listener
    });
    let executor = OpenAiExecutor::new(
        OpenAiConfig::operator(&endpoint, &endpoint).expect("config"),
        "fixture-key",
        bounds(),
        Duration::from_secs(2),
    )
    .expect("executor");
    assert_eq!(
        executor
            .edit(
                &edit("fixture scene", ImageFormat::Jpeg),
                Duration::from_secs(2)
            )
            .await
            .map(|_| ()),
        Err(ApiError::Refused)
    );
    let listener = server.join().expect("server");
    assert!(
        matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
    );
}

#[tokio::test]
async fn namespaced_operator_models_are_preserved_in_generation_and_edit_bodies() {
    let model = "operator/available-model:revision7";
    assert!(crate::config::ModelName::new(model).is_ok());
    for with_reference in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        listener.set_nonblocking(true).expect("nonblocking");
        let address = listener.local_addr().expect("address");
        let server = std::thread::spawn(move || {
            let until = std::time::Instant::now() + Duration::from_secs(3);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(std::time::Instant::now() < until, "submit not observed");
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("fixture accept: {error}"),
                }
            };
            let request = read_request_bytes(&mut stream);
            let separator = request
                .windows(4)
                .position(|part| part == b"\r\n\r\n")
                .expect("headers");
            let headers = String::from_utf8_lossy(&request[..separator]);
            let body = &request[separator + 4..];
            if with_reference {
                assert!(headers.starts_with("POST /v1/images/edits "));
                assert!(headers.contains("multipart/form-data"));
                assert!(
                    String::from_utf8_lossy(body)
                        .contains(&format!("name=\"model\"\r\n\r\n{model}\r\n"))
                );
            } else {
                assert!(headers.starts_with("POST /v1/images/generations "));
                let body: serde_json::Value = serde_json::from_slice(body).expect("JSON");
                assert_eq!(body["model"], model);
            }
            reply(&mut stream, 200, br#"{"data":[]}"#);
            listener
        });
        let bytes = minimal_jpeg(1024, 1024);
        let references = [super::ReferenceImage {
            bytes: &bytes,
            format: ImageFormat::Jpeg,
        }];
        let mut request = edit("fixture scene", ImageFormat::Jpeg);
        request.model = super::ImageEditModel::parse_model_id(model).expect("model");
        if with_reference {
            request.references = &references;
        }
        let executor = OpenAiExecutor::new(
            OpenAiConfig::loopback_fixture(address).expect("config"),
            "fixture-key",
            bounds(),
            Duration::from_secs(2),
        )
        .expect("executor");
        assert_eq!(
            executor
                .edit(&request, Duration::from_secs(2))
                .await
                .map(|_| ()),
            Err(ApiError::Refused)
        );
        let listener = server.join().expect("server");
        assert!(
            matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
        );
    }
}

async fn generate_against(status: &str, body: &str) -> Result<(), ApiError> {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let status = status
        .split_whitespace()
        .next()
        .unwrap()
        .parse::<u16>()
        .unwrap();
    let body = body.as_bytes().to_vec();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let _ = read_request(&mut stream);
        reply(&mut stream, status, &body);
    });
    let executor = OpenAiExecutor::new(
        OpenAiConfig::loopback_fixture(address).unwrap(),
        "fixture-key",
        bounds(),
        Duration::from_secs(5),
    )
    .unwrap();
    let outcome = executor
        .edit(
            &edit("Keep the same person.", ImageFormat::Png),
            Duration::from_secs(5),
        )
        .await
        .map(|_| ());
    server.join().unwrap();
    outcome
}
#[tokio::test]
async fn a_429_with_an_insufficient_quota_reason_is_quota_exhausted_not_a_rate_limit() {
    let body =
        r#"{"error":{"code":"insufficient_quota","message":"You exceeded your current quota."}}"#;
    assert_eq!(
        generate_against("429 Too Many Requests", body).await,
        Err(ApiError::QuotaExhausted)
    );
}

#[tokio::test]
async fn a_429_with_a_credit_balance_reason_is_quota_exhausted() {
    let body = r#"{"error":{"code":"credit_balance_exhausted"}}"#;
    assert_eq!(
        generate_against("429 Too Many Requests", body).await,
        Err(ApiError::QuotaExhausted)
    );
}

#[tokio::test]
async fn a_429_rate_limit_has_a_typed_rate_limited_projection() {
    let body = r#"{"error":{"code":"rate_limit_exceeded","message":"Slow down."}}"#;
    assert_eq!(
        generate_against("429 Too Many Requests", body).await,
        Err(ApiError::RateLimited)
    );
}

#[tokio::test]
async fn a_429_without_a_structured_reason_has_a_typed_rate_limited_projection() {
    assert_eq!(
        generate_against("429 Too Many Requests", "not json").await,
        Err(ApiError::RateLimited)
    );
}

#[tokio::test]
async fn a_429_with_a_null_code_takes_the_quota_reason_from_the_error_type() {
    let body = r#"{"error":{"code":null,"type":"insufficient_quota","message":"You exceeded your current quota."}}"#;
    assert_eq!(
        generate_against("429 Too Many Requests", body).await,
        Err(ApiError::QuotaExhausted)
    );
}

#[tokio::test]
async fn a_429_whose_code_is_a_rate_limit_ignores_the_error_type() {
    let body = r#"{"error":{"code":"rate_limit_exceeded","type":"insufficient_quota"}}"#;
    assert_eq!(
        generate_against("429 Too Many Requests", body).await,
        Err(ApiError::RateLimited)
    );
}

#[tokio::test]
async fn observed_declines_keep_exact_response_and_only_closed_submit_proof() {
    use super::{DeclineContract, SubmissionEvidence};
    for (status, body, error, proof) in [
        (
            429,
            r#"{"error":{"code":"rate_limit_exceeded"}}"#,
            ApiError::RateLimited,
            Some(DeclineContract::OpenAiImages429V1),
        ),
        (
            429,
            r#"{"error":{"code":"insufficient_quota"}}"#,
            ApiError::QuotaExhausted,
            None,
        ),
        (429, r#"{"error":{}}"#, ApiError::RateLimited, None),
        (
            429,
            r#"{"error":{"code":"rate_limit_exceeded","accepted":{"id":"job"}}}"#,
            ApiError::RateLimited,
            None,
        ),
        (
            400,
            r#"{"error":{"code":"moderation_blocked"}}"#,
            ApiError::Refused,
            None,
        ),
        (
            401,
            r#"{"error":{"message":"unauthorized"}}"#,
            ApiError::Unauthorized,
            None,
        ),
        (
            503,
            r#"{"error":{"message":"unavailable"}}"#,
            ApiError::Provider(503),
            None,
        ),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        let address = listener.local_addr().expect("address");
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            assert!(read_request(&mut stream).starts_with("POST /v1/images/generations "));
            reply(&mut stream, status, body.as_bytes());
        });
        let executor = OpenAiExecutor::new(
            OpenAiConfig::loopback_fixture(address).expect("config"),
            "fixture-key",
            bounds(),
            Duration::from_secs(5),
        )
        .expect("executor");
        let failure = executor
            .edit_observed(
                &edit("private fixture prompt", ImageFormat::Png),
                Duration::from_secs(5),
            )
            .await
            .expect_err("decline");
        assert_eq!(failure.error, error);
        assert_eq!(failure.retryable_submit_decline(), proof);
        assert_eq!(
            failure.submission,
            proof
                .map(SubmissionEvidence::RejectedSubmit)
                .unwrap_or(SubmissionEvidence::Unknown)
        );
        let receipt = failure.response.expect("actual complete HTTP response");
        assert_eq!(receipt.status, status);
        assert_eq!(receipt.body, body.as_bytes());
        assert!(receipt.retry_after.raw_values().is_empty());
        assert!(!format!("{receipt:?}").contains("unavailable"));
        server.join().expect("fixture");
    }
}

#[test]
fn failed_artifact_can_preserve_complete_reported_usage_without_inventing_missing_units() {
    use super::openai::reported_image_usage;
    let usage = reported_image_usage(br#"{"usage":{"input_tokens_details":{"text_tokens":7,"image_tokens":3},"output_tokens":11},"data":[{"b64_json":"invalid"}]}"#).expect("complete reported usage");
    assert_eq!(usage.text_input_tokens, 7);
    assert_eq!(usage.image_input_tokens, 3);
    assert_eq!(usage.output_tokens, 11);
    assert!(
        reported_image_usage(
            br#"{"usage":{"input_tokens_details":{"text_tokens":7},"output_tokens":11}}"#
        )
        .is_none()
    );
}
