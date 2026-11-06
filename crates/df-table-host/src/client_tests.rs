use axum::http::StatusCode;
use tokio::io::AsyncReadExt;

use crate::client::{ClientError, StreamedPost, get, get_with_limit, post_json};
use crate::limits::{STREAM_ANSWER_TIMEOUT, STREAM_STEP_TIMEOUT};
use crate::test_support::{STARTER_FILES, canned_server, closed_port, start};

fn http(head_lines: &str, body: &str) -> Vec<u8> {
    format!("HTTP/1.1 200 OK\r\n{head_lines}\r\n{body}").into_bytes()
}

#[tokio::test]
async fn a_complete_answer_is_returned_with_its_status_and_body() {
    let port = canned_server(http("Content-Length: 5\r\nConnection: close\r\n", "hello")).await;
    let response = get(port, "/").await.expect("answers");
    assert_eq!(
        (response.status, response.body.as_str()),
        (StatusCode::OK, "hello")
    );
}

#[tokio::test]
async fn an_answer_with_no_length_is_read_to_the_end_of_the_connection() {
    let port = canned_server(http("Connection: close\r\n", "until close")).await;
    assert_eq!(get(port, "/").await.expect("answers").body, "until close");
}

#[tokio::test]
async fn an_answer_cut_short_of_its_length_is_an_error_not_a_shorter_body() {
    let port = canned_server(http("Content-Length: 10\r\n", "abc")).await;
    assert_eq!(get(port, "/").await, Err(ClientError::Truncated));
}

#[tokio::test]
async fn a_body_cut_short_is_truncated_even_when_its_bytes_are_not_utf8() {
    // Four bytes of a ten-byte body: read as text they would become twelve bytes of replacement
    // characters and pass for a complete body.
    let mut answer = b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\n".to_vec();
    answer.extend_from_slice(&[0xFF; 4]);
    let port = canned_server(answer).await;
    assert_eq!(get(port, "/").await, Err(ClientError::Truncated));
}

#[tokio::test]
async fn a_chunked_answer_is_refused_however_its_header_is_spaced_or_cased() {
    let port = canned_server(http(
        "transfer-encoding:Chunked\r\n",
        "3\r\nabc\r\n0\r\n\r\n",
    ))
    .await;
    assert_eq!(get(port, "/").await, Err(ClientError::InvalidResponse));
}

#[tokio::test]
async fn a_chunked_answer_is_refused_because_the_host_never_sends_one() {
    let port = canned_server(http(
        "Transfer-Encoding: chunked\r\n",
        "3\r\nabc\r\n0\r\n\r\n",
    ))
    .await;
    assert_eq!(get(port, "/").await, Err(ClientError::InvalidResponse));
}

#[tokio::test]
async fn text_that_is_not_http_is_an_invalid_response() {
    let port = canned_server(b"not http at all".to_vec()).await;
    assert_eq!(get(port, "/").await, Err(ClientError::InvalidResponse));
    let port = canned_server(b"HTTP/1.1 abc\r\n\r\n".to_vec()).await;
    assert_eq!(get(port, "/").await, Err(ClientError::InvalidResponse));
}

#[tokio::test]
async fn a_status_outside_the_three_digit_http_range_is_an_invalid_response() {
    for line in ["HTTP/1.1 0 Zero", "HTTP/1.1 99 Low", "HTTP/1.1 1000 High"] {
        let answer = format!("{line}\r\nContent-Length: 0\r\n\r\n");
        let port = canned_server(answer.into_bytes()).await;
        assert_eq!(
            get(port, "/").await,
            Err(ClientError::InvalidResponse),
            "{line}"
        );
    }
}

#[tokio::test]
async fn an_answer_over_the_cap_is_too_large() {
    let port = canned_server(http("Connection: close\r\n", &"x".repeat(500))).await;
    assert_eq!(
        get_with_limit(port, "/", 100).await,
        Err(ClientError::TooLarge)
    );
}

#[tokio::test]
async fn an_answer_exactly_at_the_cap_is_accepted() {
    let answer = http("Content-Length: 5\r\n", "hello");
    let limit = answer.len();
    let port = canned_server(answer).await;
    assert_eq!(
        get_with_limit(port, "/", limit)
            .await
            .expect("answers")
            .body,
        "hello"
    );
}

#[tokio::test]
async fn a_port_nothing_listens_on_is_not_serving() {
    let port = closed_port().await;
    assert_eq!(get(port, "/").await, Err(ClientError::NotServing { port }));
    assert!(
        ClientError::NotServing { port }
            .to_string()
            .contains("df-table serve")
    );
}

#[tokio::test]
async fn a_real_host_answers_a_get_and_a_post() {
    let running = start(&STARTER_FILES).await;
    let canvas = get(running.port, "/canvas.json").await.expect("answers");
    assert_eq!(canvas.status, StatusCode::OK);
    let rejected = post_json(running.port, "/dev/effect", "{}")
        .await
        .expect("answers");
    assert_eq!(rejected.status, StatusCode::BAD_REQUEST);
}

/// The host answers an upload only after it has stored the file, and nothing arrives meanwhile, so
/// the wait for that answer has its own fixed bound, longer than one sending step.
#[tokio::test]
async fn a_streamed_upload_waits_for_its_answer_up_to_the_answer_bound() {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("binds an ephemeral port");
    let port = listener.local_addr().expect("has an address").port();
    let _host = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accepts");
        // Reads the whole upload and never answers, like a host still storing the file.
        let mut received = Vec::new();
        let _ = stream.read_to_end(&mut received).await;
    });
    let mut post = StreamedPost::begin(port, "/dev/media/add", 2)
        .await
        .expect("connects");
    post.send(b"{}").await.expect("sends");
    // Only the answer's deadline is pending from here on, so paused time jumps straight to it.
    tokio::time::pause();
    let started = tokio::time::Instant::now();
    assert_eq!(post.finish().await, Err(ClientError::TimedOut));
    let waited = started.elapsed();
    assert!(STREAM_ANSWER_TIMEOUT > STREAM_STEP_TIMEOUT);
    assert!(waited >= STREAM_ANSWER_TIMEOUT, "gave up after {waited:?}");
}
