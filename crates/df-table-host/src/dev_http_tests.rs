//! The dev reply envelope, the shared reply helper and the body reader.

use axum::http::StatusCode;
use axum::response::Response;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::dev_http::{BodyCause, BodyError, DevReply, RouteError, parse_body, reply};

/// A route module's error, declared the way a module on the envelope declares it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "snake_case")]
enum ProbeError {
    NotStarted,
    InvalidBody(BodyError),
}

impl std::fmt::Display for ProbeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotStarted => formatter.write_str("the probe did not start"),
            Self::InvalidBody(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for ProbeError {}

impl From<BodyError> for ProbeError {
    fn from(error: BodyError) -> Self {
        Self::InvalidBody(error)
    }
}

impl RouteError for ProbeError {
    fn status(&self) -> StatusCode {
        match self {
            Self::NotStarted => StatusCode::SERVICE_UNAVAILABLE,
            Self::InvalidBody(_) => StatusCode::BAD_REQUEST,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProbeRequest {
    #[expect(dead_code, reason = "the field is only read to check the body's shape")]
    name: String,
}

async fn body_json(response: Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("reads the body");
    serde_json::from_slice(&bytes).expect("the body is JSON")
}

#[test]
fn a_done_reply_is_tagged_done_with_its_value() {
    let reply: DevReply<u32, ProbeError> = DevReply::from_result(Ok(7));
    assert_eq!(
        serde_json::to_value(&reply).expect("writes"),
        json!({"outcome": "done", "value": 7})
    );
}

#[test]
fn a_rejected_reply_carries_the_typed_error_tagged_error_and_its_sentence() {
    let reply: DevReply<u32, ProbeError> = DevReply::rejected(ProbeError::NotStarted);
    let wire = serde_json::to_value(&reply).expect("writes");
    assert_eq!(
        wire,
        json!({
            "outcome": "rejected",
            "error": {"error": "not_started"},
            "message": "the probe did not start"
        })
    );
    let read: DevReply<u32, ProbeError> = serde_json::from_value(wire).expect("reads back");
    assert_eq!(read, reply);
}

#[tokio::test]
async fn the_shared_reply_answers_200_with_done_and_the_errors_status_with_rejected() {
    let done = reply::<u32, ProbeError>(Ok(7));
    assert_eq!(done.status(), StatusCode::OK);
    assert_eq!(body_json(done).await["outcome"], "done");

    let refused = reply::<u32, ProbeError>(Err(ProbeError::NotStarted));
    assert_eq!(refused.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = body_json(refused).await;
    assert_eq!(body["outcome"], "rejected");
    assert_eq!(body["error"]["error"], "not_started");
}

#[test]
fn a_body_that_is_not_json_is_a_syntax_error_with_its_place() {
    let error = parse_body::<ProbeRequest>(b"{ not json").expect_err("refused");
    assert_eq!(error.cause, BodyCause::Syntax);
    assert_eq!(error.line, 1);
    assert!(error.column > 0);
    assert!(
        error
            .to_string()
            .starts_with("the request body is not valid: "),
        "{error}"
    );
}

#[test]
fn a_body_of_the_wrong_shape_is_a_data_error_and_an_empty_one_is_truncated() {
    let unknown = parse_body::<ProbeRequest>(br#"{"name":"a","extra":1}"#).expect_err("refused");
    assert_eq!(unknown.cause, BodyCause::Data);
    assert!(unknown.detail.contains("extra"), "{}", unknown.detail);
    let empty = parse_body::<ProbeRequest>(b"").expect_err("refused");
    assert_eq!(empty.cause, BodyCause::Truncated);
}

#[tokio::test]
async fn a_body_error_becomes_the_module_error_and_travels_whole_on_the_wire() {
    let result: Result<u32, ProbeError> = parse_body::<ProbeRequest>(b"")
        .map(|_| 0)
        .map_err(ProbeError::from);
    let answer = reply(result);
    assert_eq!(answer.status(), StatusCode::BAD_REQUEST);
    let body = body_json(answer).await;
    assert_eq!(body["error"]["error"], "invalid_body");
    assert_eq!(body["error"]["cause"], "truncated");
    let read: ProbeError = serde_json::from_value(body["error"].clone()).expect("reads back");
    assert!(matches!(
        read,
        ProbeError::InvalidBody(BodyError {
            cause: BodyCause::Truncated,
            ..
        })
    ));
}

#[test]
fn body_cause_keys_are_its_serde_names() {
    for cause in BodyCause::ALL {
        assert_eq!(
            serde_json::to_value(cause).expect("writes"),
            json!(cause.key())
        );
        assert_eq!(BodyCause::from_key(cause.key()), Some(cause));
    }
    assert_eq!(BodyCause::from_key("Syntax"), None);
}
