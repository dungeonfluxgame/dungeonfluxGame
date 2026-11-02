use crate::client::{ClientError, get, get_with_limit, post_json};
use crate::test_support::{STARTER_FILES, canned_server, closed_port, start};

fn http(head_lines: &str, body: &str) -> Vec<u8> {
    format!("HTTP/1.1 200 OK\r\n{head_lines}\r\n{body}").into_bytes()
}

#[tokio::test]
async fn a_complete_answer_is_returned_with_its_status_and_body() {
    let port = canned_server(http("Content-Length: 5\r\nConnection: close\r\n", "hello")).await;
    let response = get(port, "/").await.expect("answers");
    assert_eq!((response.status, response.body.as_str()), (200, "hello"));
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
async fn a_chunked_answer_is_refused_because_the_host_never_sends_one() {
    let port = canned_server(http(
        "Transfer-Encoding: chunked\r\n",
        "3\r\nabc\r\n0\r\n\r\n",
    ))
    .await;
    assert_eq!(get(port, "/").await, Err(ClientError::BadResponse));
}

#[tokio::test]
async fn text_that_is_not_http_is_a_bad_response() {
    let port = canned_server(b"not http at all".to_vec()).await;
    assert_eq!(get(port, "/").await, Err(ClientError::BadResponse));
    let port = canned_server(b"HTTP/1.1 abc\r\n\r\n".to_vec()).await;
    assert_eq!(get(port, "/").await, Err(ClientError::BadResponse));
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
    assert_eq!(canvas.status, 200);
    let rejected = post_json(running.port, "/dev/effect", "{}")
        .await
        .expect("answers");
    assert_eq!(rejected.status, 400);
}
