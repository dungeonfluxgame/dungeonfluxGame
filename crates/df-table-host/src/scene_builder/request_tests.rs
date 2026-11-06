//! The scene builder's wire: every closed key is its serde name, every error answers its status,
//! and the inbound bodies are strict and typed.

use axum::http::StatusCode;
use df_table::lookahead::Expectation;
use serde::Serialize;
use serde_json::json;

use super::lookahead::request::{LookaheadRequest, LookaheadSettingName};
use super::request::{
    BuildEnd, BuildRequest, DevMoment, DevStage, LookaheadTake, RunRequest, SceneBuilderError,
    WorkbenchCause,
};
use crate::dev_http::{BodyCause, BodyError, RouteError};

fn serde_name(value: &impl Serialize) -> String {
    serde_json::to_value(value)
        .expect("serializes")
        .as_str()
        .expect("a unit variant is a string")
        .to_owned()
}

#[test]
fn every_closed_key_is_its_serde_name_and_reads_back() {
    for stage in DevStage::ALL {
        assert_eq!(stage.key(), serde_name(&stage));
        assert_eq!(DevStage::from_key(stage.key()), Some(stage));
    }
    for moment in DevMoment::ALL {
        assert_eq!(moment.key(), serde_name(&moment));
        assert_eq!(DevMoment::from_key(moment.key()), Some(moment));
    }
    for end in BuildEnd::ALL {
        assert_eq!(end.key(), serde_name(&end));
        assert_eq!(BuildEnd::from_key(end.key()), Some(end));
    }
    for take in LookaheadTake::ALL {
        assert_eq!(take.key(), serde_name(&take));
        assert_eq!(LookaheadTake::from_key(take.key()), Some(take));
    }
    for cause in WorkbenchCause::ALL {
        assert_eq!(cause.key(), serde_name(&cause));
        assert_eq!(WorkbenchCause::from_key(cause.key()), Some(cause));
    }
    for setting in LookaheadSettingName::ALL {
        assert_eq!(setting.key(), serde_name(&setting));
        assert_eq!(LookaheadSettingName::from_key(setting.key()), Some(setting));
    }
}

fn every_error() -> Vec<(SceneBuilderError, StatusCode)> {
    vec![
        (
            SceneBuilderError::InvalidBody(BodyError {
                cause: BodyCause::Data,
                line: 1,
                column: 1,
                detail: "unknown field".to_owned(),
            }),
            StatusCode::BAD_REQUEST,
        ),
        (SceneBuilderError::NothingBuilt, StatusCode::BAD_REQUEST),
        (
            SceneBuilderError::InvalidSetting {
                setting: LookaheadSettingName::Depth,
                value: 0,
            },
            StatusCode::BAD_REQUEST,
        ),
        (
            SceneBuilderError::InvalidLookaheadStage {
                stage: DevStage::Plate,
            },
            StatusCode::BAD_REQUEST,
        ),
        (
            SceneBuilderError::InvalidLikelihood {
                kind: Expectation::ExitHint,
                likelihood_permille: None,
            },
            StatusCode::BAD_REQUEST,
        ),
        (
            SceneBuilderError::Workbench {
                cause: WorkbenchCause::World,
                detail: "the dev table refused a change".to_owned(),
            },
            StatusCode::INTERNAL_SERVER_ERROR,
        ),
        (
            SceneBuilderError::NotStarted {
                cause: WorkbenchCause::Purposes,
                detail: "a stage's purpose is not a name".to_owned(),
            },
            StatusCode::SERVICE_UNAVAILABLE,
        ),
        (SceneBuilderError::Stopped, StatusCode::SERVICE_UNAVAILABLE),
    ]
}

#[test]
fn an_errors_key_is_its_serde_name_and_it_answers_its_status() {
    for (error, status) in every_error() {
        let wire = serde_json::to_value(&error).expect("serializes");
        assert_eq!(wire["error"], error.key(), "{error:?}");
        assert_eq!(error.status(), status, "{error:?}");
        let read: SceneBuilderError = serde_json::from_value(wire).expect("reads back");
        assert_eq!(read, error);
    }
}

#[test]
fn a_refused_setting_or_likelihood_says_what_is_allowed() {
    let depth = SceneBuilderError::InvalidSetting {
        setting: LookaheadSettingName::Depth,
        value: 0,
    };
    assert!(depth.to_string().contains("lookahead_depth"), "{depth}");
    let enabled = SceneBuilderError::InvalidSetting {
        setting: LookaheadSettingName::Enabled,
        value: 2,
    };
    assert_eq!(enabled.to_string(), "lookahead_enabled is 0 or 1, not 2");
    let wild = SceneBuilderError::InvalidLikelihood {
        kind: Expectation::ExitHint,
        likelihood_permille: Some(1_500),
    };
    assert_eq!(
        wild.to_string(),
        "a likelihood is 0 to 1000 thousandths, not 1500"
    );
}

#[test]
fn the_bodies_refuse_unknown_fields_and_untyped_values() {
    for body in [
        json!({"place": "quay", "surprise": 1}),
        json!({"place": "  "}),
        json!({"place": "quay", "fail": "ready"}),
    ] {
        assert!(
            serde_json::from_value::<BuildRequest>(body.clone()).is_err(),
            "{body}"
        );
    }
    assert!(serde_json::from_value::<RunRequest>(json!({"seconds": 1, "moment": "soon"})).is_err());
    for body in [
        json!({"action": "target", "place": "\u{7}"}),
        json!({"action": "set", "setting": "lookahead_depth", "value": 2}),
    ] {
        assert!(
            serde_json::from_value::<LookaheadRequest>(body.clone()).is_err(),
            "{body}"
        );
    }
}
