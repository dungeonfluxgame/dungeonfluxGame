//! What crosses the route: the strict body, the typed errors on the wire and their statuses.

use std::path::PathBuf;

use axum::http::StatusCode;

use super::backend::BackendError;
use super::model::{Aspect, GenerateRequestError, Purpose};
use super::request::{
    GenerateError, GenerateReply, GenerateRequest, JobState, JobView, PictureCause, Sidecar,
};
use crate::dev_http::{BodyCause, RouteError, parse_body};
use crate::locate::IoCause;

fn every_error() -> Vec<GenerateError> {
    let body = parse_body::<GenerateRequest>(b"{}").expect_err("no prompt");
    vec![
        GenerateError::InvalidBody(body),
        GenerateError::InvalidRequest(GenerateRequestError::EmptyPrompt),
        GenerateError::InvalidRequest(GenerateRequestError::PromptTooLong {
            chars: 20_000,
            maximum_chars: 15_000,
        }),
        GenerateError::InvalidRequest(GenerateRequestError::InvalidName {
            name: "a b".to_owned(),
        }),
        GenerateError::NameBusy {
            name: "hall".to_owned(),
        },
        GenerateError::Backend {
            job: None,
            backend: BackendError::NoLogin {
                home: PathBuf::from("home"),
            },
        },
        GenerateError::Backend {
            job: Some(3),
            backend: BackendError::Timeout,
        },
        GenerateError::InvalidPicture {
            job: 3,
            cause: PictureCause::NotOfType {
                mime: "image/png".to_owned(),
            },
        },
        GenerateError::StoreFailed {
            job: 3,
            detail: "the library is full".to_owned(),
        },
        GenerateError::Stopped { job: None },
    ]
}

#[test]
fn a_body_is_strict_and_typed() {
    let request: GenerateRequest =
        parse_body(br#"{"prompt":"a hall","name":"hall","purpose":"portrait","aspect":"3:4"}"#)
            .expect("reads");
    assert_eq!(request.purpose, Some(Purpose::Portrait));
    assert_eq!(request.aspect, Some(Aspect::PORTRAIT));
    assert!(!request.wait);
    for body in [
        r#"{"prompt":"a","name":"hall","colour":"red"}"#,
        r#"{"prompt":"a","name":"hall","purpose":"poster"}"#,
        r#"{"prompt":"a","name":"hall","aspect":"wide"}"#,
        r#"{"prompt":"a","name":"hall","aspect":"1:9"}"#,
    ] {
        let error = parse_body::<GenerateRequest>(body.as_bytes()).expect_err(body);
        assert_eq!(error.cause, BodyCause::Data, "{body}");
    }
}

#[test]
fn an_error_key_is_its_serde_tag_and_it_reads_back() {
    for error in every_error() {
        let json = serde_json::to_value(&error).expect("serializes");
        assert_eq!(json["error"], error.key(), "{json}");
        let read: GenerateError = serde_json::from_value(json).expect("reads back");
        assert_eq!(read, error);
    }
}

#[test]
fn a_request_error_key_is_its_serde_tag() {
    for error in [
        GenerateRequestError::EmptyPrompt,
        GenerateRequestError::PromptTooLong {
            chars: 2,
            maximum_chars: 1,
        },
        GenerateRequestError::InvalidName {
            name: "a b".to_owned(),
        },
    ] {
        let json = serde_json::to_value(&error).expect("serializes");
        assert_eq!(json["cause"], error.key(), "{json}");
    }
}

#[test]
fn a_picture_cause_key_is_its_serde_name() {
    for cause in [
        PictureCause::Empty,
        PictureCause::NotOfType {
            mime: "image/png".to_owned(),
        },
        PictureCause::NoSize,
    ] {
        let json = serde_json::to_value(&cause).expect("serializes");
        assert_eq!(json["cause"], cause.key(), "{json}");
    }
}

#[test]
fn each_error_answers_the_status_of_its_kind_of_failure() {
    let statuses: Vec<StatusCode> = every_error().iter().map(RouteError::status).collect();
    assert_eq!(
        statuses,
        vec![
            StatusCode::BAD_REQUEST,
            StatusCode::BAD_REQUEST,
            StatusCode::BAD_REQUEST,
            StatusCode::BAD_REQUEST,
            StatusCode::CONFLICT,
            StatusCode::SERVICE_UNAVAILABLE,
            StatusCode::GATEWAY_TIMEOUT,
            StatusCode::BAD_GATEWAY,
            StatusCode::INTERNAL_SERVER_ERROR,
            StatusCode::INTERNAL_SERVER_ERROR,
        ]
    );
}

#[test]
fn a_failed_job_and_a_sidecar_read_back_from_their_json() {
    let failed = JobView {
        job: 4,
        name: "hall".to_owned(),
        deadline_ms: 900_000,
        elapsed_ms: 12,
        state: JobState::Failed {
            error: GenerateError::Backend {
                job: Some(4),
                backend: BackendError::Interrupted,
            },
            message: "interrupted".to_owned(),
        },
    };
    let reply: GenerateReply<Vec<JobView>> = GenerateReply::Done {
        value: vec![failed],
    };
    let json = serde_json::to_value(&reply).expect("serializes");
    assert_eq!(json["outcome"], "done");
    assert_eq!(json["value"][0]["state"], "failed");
    assert_eq!(json["value"][0]["deadline_ms"], 900_000);
    assert_eq!(json["value"][0]["error"]["error"], "backend");
    assert_eq!(json["value"][0]["error"]["cause"], "interrupted");
    let read: GenerateReply<Vec<JobView>> = serde_json::from_value(json).expect("reads back");
    assert_eq!(read, reply);

    let sidecar = Sidecar::NotWritten {
        path: PathBuf::from("hall.provenance.json"),
        cause: IoCause::PermissionDenied,
    };
    let json = serde_json::to_value(&sidecar).expect("serializes");
    assert_eq!(json["not_written"]["cause"], "permission_denied");
    assert_eq!(
        serde_json::from_value::<Sidecar>(json).expect("reads back"),
        sidecar
    );
}

#[test]
fn the_cause_inside_an_error_is_always_the_cause_field_beside_the_error_key() {
    let shapes = [
        (
            GenerateError::Backend {
                job: Some(3),
                backend: BackendError::NoLogin {
                    home: PathBuf::from("home"),
                },
            },
            serde_json::json!({"error": "backend", "job": 3, "cause": "no_login", "home": "home"}),
        ),
        (
            GenerateError::Backend {
                job: None,
                backend: BackendError::Timeout,
            },
            serde_json::json!({"error": "backend", "job": null, "cause": "timeout"}),
        ),
        (
            GenerateError::InvalidPicture {
                job: 3,
                cause: PictureCause::NoSize,
            },
            serde_json::json!({"error": "invalid_picture", "job": 3, "cause": "no_size"}),
        ),
        (
            GenerateError::InvalidRequest(GenerateRequestError::EmptyPrompt),
            serde_json::json!({"error": "invalid_request", "cause": "empty_prompt"}),
        ),
    ];
    for (error, shape) in shapes {
        assert_eq!(serde_json::to_value(&error).expect("serializes"), shape);
        assert_eq!(
            serde_json::from_value::<GenerateError>(shape).expect("reads"),
            error
        );
    }
}
