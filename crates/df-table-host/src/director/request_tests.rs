//! The Director's wire: its error's keys are its serde names, its statuses, and the shape of a
//! refusal in the shared envelope.

use axum::http::StatusCode;
use df_table::director::error::DirectorError as DecisionError;
use serde_json::json;

use super::request::{ActReply, DirectorAct, DirectorError, DirectorReply, StartKind};
use crate::dev_http::{BodyCause, BodyError, DevReply, RouteError};

fn every_error() -> [DirectorError; 4] {
    [
        DirectorError::InvalidBody(BodyError {
            cause: BodyCause::Data,
            line: 1,
            column: 2,
            detail: "unknown variant".to_owned(),
        }),
        DirectorError::Refused {
            cause: DecisionError::NoTargets,
        },
        DirectorError::InvalidAdvance { by_ms: 1 },
        DirectorError::NoStage {
            detail: "no fixture is shown".to_owned(),
        },
    ]
}

#[test]
fn a_start_kinds_key_is_its_serde_name_and_reads_back() {
    for kind in StartKind::ALL {
        assert_eq!(serde_json::to_value(kind).expect("serializes"), kind.key());
        assert_eq!(StartKind::from_key(kind.key()), Some(kind));
    }
    assert_eq!(StartKind::from_key("Scene"), None);
}

#[test]
fn an_errors_key_is_its_serde_name_and_every_refusal_is_the_callers_mistake() {
    for error in every_error() {
        let wire = serde_json::to_value(&error).expect("serializes");
        assert_eq!(wire["error"], error.key(), "{error:?}");
        assert_eq!(error.status(), StatusCode::BAD_REQUEST, "{error:?}");
        let read: DirectorError = serde_json::from_value(wire).expect("reads back");
        assert_eq!(read, error);
    }
}

#[test]
fn a_refusal_by_the_director_carries_its_typed_cause_in_the_envelope() {
    let reply: DirectorReply<ActReply> = DevReply::rejected(DirectorError::Refused {
        cause: DecisionError::ZoomOutOfRange { zoom_percent: 99 },
    });
    let wire = serde_json::to_value(&reply).expect("serializes");
    assert_eq!(wire["outcome"], "rejected");
    assert_eq!(
        wire["error"],
        json!({"error": "refused", "cause": {"error": "zoom_out_of_range", "zoom_percent": 99}})
    );
    assert!(
        wire["message"]
            .as_str()
            .is_some_and(|message| message.contains("not 99")),
        "{wire}"
    );
}

#[test]
fn a_verb_reads_its_typed_fields_and_refuses_anything_else() {
    let read: DirectorAct = serde_json::from_value(json!({
        "verb": "focus",
        "element": "finn",
        "reason": "hit",
        "zoom_percent": 250,
        "over_ms": 1_500,
        "hold_ms": 4_000,
    }))
    .expect("a focus");
    assert!(matches!(read, DirectorAct::Focus { .. }));
    for body in [
        json!({"verb": "focus", "element": "finn", "zoom": 250}),
        json!({"verb": "advance", "ms": 10}),
        json!({"verb": "view", "edit": {"edit": "beat", "reveal": true, "ms": 10}}),
    ] {
        assert!(
            serde_json::from_value::<DirectorAct>(body.clone()).is_err(),
            "{body} uses a field the verb does not take"
        );
    }
}
