use axum::http::StatusCode;
use df_playground::controls::{Controls, ControlsChangeReply, ControlsReply};

use super::SceneApplyError;
use super::request::{ChangeReply, PlaygroundChangeError};
use crate::dev_http::{BodyCause, BodyError, DevReply, RouteError};

fn every_error() -> Vec<PlaygroundChangeError> {
    let detail = || "why".to_owned();
    vec![
        PlaygroundChangeError::InvalidBody(BodyError {
            cause: BodyCause::Syntax,
            line: 1,
            column: 1,
            detail: detail(),
        }),
        PlaygroundChangeError::InvalidChange { detail: detail() },
        PlaygroundChangeError::FixtureChanged { detail: detail() },
        PlaygroundChangeError::FileMissing { detail: detail() },
        PlaygroundChangeError::FileTooLarge { detail: detail() },
        PlaygroundChangeError::FileUnreadable { detail: detail() },
    ]
}

#[test]
fn a_scene_refusal_reads_as_the_change_error_whose_status_it_carries() {
    for error in every_error() {
        if matches!(error, PlaygroundChangeError::InvalidBody(_)) {
            continue;
        }
        let scene = SceneApplyError {
            status: error.status(),
            message: error.to_string(),
        };
        assert_eq!(PlaygroundChangeError::from(scene), error);
    }
}

#[test]
fn every_change_error_round_trips_tagged_by_its_error_key() {
    for error in every_error() {
        let written = serde_json::to_value(&error).expect("writes");
        assert!(written["error"].is_string(), "{written}");
        let read: PlaygroundChangeError = serde_json::from_value(written).expect("reads");
        assert_eq!(read, error);
    }
}

#[test]
fn the_page_reads_the_hosts_change_reply_both_ways() {
    let controls = ControlsReply {
        revision: 3,
        boot_id: "run".to_owned(),
        controls: Controls::default(),
        scene: Default::default(),
    };
    let done: ChangeReply = DevReply::Done {
        value: controls.clone(),
    };
    let text = serde_json::to_string(&done).expect("writes");
    let read: ControlsChangeReply = serde_json::from_str(&text).expect("the page reads it");
    assert_eq!(read.into_result(), Ok(controls));
    let refused: ChangeReply = DevReply::rejected(PlaygroundChangeError::FixtureChanged {
        detail: "try again".to_owned(),
    });
    let text = serde_json::to_string(&refused).expect("writes");
    let read: ControlsChangeReply = serde_json::from_str(&text).expect("the page reads it");
    assert_eq!(read.into_result(), Err("try again".to_owned()));
    assert_eq!(
        PlaygroundChangeError::FixtureChanged {
            detail: String::new()
        }
        .status(),
        StatusCode::CONFLICT
    );
}
