use df_scene_layout::{CharacterCue, CueKind};

use super::CastCommand;
use super::request::{CameraMoved, CameraRequest, CastError, CastList, CastReply, Cued};
use super::routes::path;
use crate::command::{Command, parse};
use crate::dev_http::{DevReply, RouteError};
use crate::shown_scene::ShownSceneError;
use crate::test_support::layout_of;

fn words(text: &str) -> Vec<String> {
    text.split_whitespace().map(str::to_owned).collect()
}

fn cue(character: &str, kind: CueKind) -> Command {
    Command::Cast(CastCommand::Cue(CharacterCue {
        character: character.to_owned(),
        kind,
    }))
}

#[test]
fn the_cast_grammar() {
    assert_eq!(
        parse(&words("scene cast")).unwrap(),
        Command::Cast(CastCommand::List)
    );
    assert_eq!(
        parse(&words("scene cue elf think")).unwrap(),
        cue("elf", CueKind::Thinking)
    );
    assert_eq!(
        parse(&words("scene cue elf speak The fast road, then.")).unwrap(),
        cue(
            "elf",
            CueKind::Speaking {
                line: "The fast road, then.".to_owned()
            }
        )
    );
    assert_eq!(
        parse(&words("scene cue elf quiet")).unwrap(),
        cue("elf", CueKind::Quiet)
    );
    assert_eq!(
        parse(&words("scene camera-on innkeeper shot=face over=800")).unwrap(),
        Command::Cast(CastCommand::Camera {
            character: "innkeeper".to_owned(),
            shot: Some(df_scene_layout::Shot::Face),
            over_ms: Some(800),
        })
    );
    for bad in [
        "scene cast extra",
        "scene cue elf",
        "scene cue elf speak",
        "scene cue elf shout",
        "scene camera-on",
        "scene camera-on innkeeper shot=knees",
        "scene camera-on innkeeper over=soon",
    ] {
        assert!(parse(&words(bad)).is_err(), "{bad} should be refused");
    }
}

#[test]
fn a_cue_is_the_same_json_an_engine_posts() {
    let posted: CharacterCue =
        serde_json::from_str(r#"{"character":"elf","kind":"speaking","line":"Hello."}"#).unwrap();
    let Command::Cast(CastCommand::Cue(parsed)) =
        parse(&words("scene cue elf speak Hello.")).unwrap()
    else {
        panic!("not a cue");
    };
    assert_eq!(posted, parsed);
}

#[tokio::test]
async fn the_cast_and_the_camera_follow_the_backdrop_the_scene_shows() {
    use crate::test_support::{STARTER_FILES, start};
    let running = start(&STARTER_FILES).await;
    let layouts = running.playground_path("assets/layouts");
    for (picture, name) in [("first.png", "elf"), ("second.png", "dwarf")] {
        running.write_asset(&format!("media/{picture}"), picture);
        crate::locate::write_layout(&layouts, picture, &layout_of(picture, name)).unwrap();
    }
    running.write_asset(
        "fixtures/one.json",
        concat!(
            r#"{"name": "One", "tv": {"place": "Here", "status": {"kind": "ok"}, "elements": [], "#,
            r#""backdrop": {"media": "first.png", "caption": null}}, "#,
            r#""phone": {"hero": "Pip", "status": {"kind": "ok"}}}"#
        ),
    );
    let elf: df_content::persona::Persona = serde_json::from_value(serde_json::json!({
        "schema": 1, "id": "elf", "display_name": "Sylwen", "job": "Scout of the pass"
    }))
    .unwrap();
    crate::persona::write_persona(&running.playground_path("assets/personas"), &elf).unwrap();
    let port = running.port;
    let run = |line: &'static str| async move { crate::dev::run_dev(port, &words(line)).await };
    let first = run("scene cast").await.unwrap();
    assert!(first.contains("elf") && !first.contains("dwarf"), "{first}");
    assert!(first.contains("elf Sylwen (Scout of the pass) ["), "{first}");
    let listed: CastReply<CastList> = crate::dev_client::get_json(port, path::CAST).await.unwrap();
    let DevReply::Done { value } = listed else {
        panic!("the cast is listed");
    };
    assert_eq!(value.personas.len(), 1);
    assert_eq!(value.personas[0].job, "Scout of the pass");

    run("scene image tv-backdrop second.png").await.unwrap();
    let second = run("scene cast").await.unwrap();
    assert!(
        second.contains("dwarf") && !second.contains("elf"),
        "{second}"
    );
    assert!(!second.contains("Scout"), "{second}");
    run("scene camera region:dwarf").await.unwrap();
    run("scene camera-on dwarf").await.unwrap();
    let refused = run("scene camera-on elf").await.unwrap_err();
    assert!(
        matches!(
            &refused,
            crate::dev::DevError::Rejected {
                error: crate::dev::ReplyError::Cast(CastError::Shown {
                    cause: ShownSceneError::UnknownCharacter { character, .. }
                }),
                ..
            } if character == "elf"
        ),
        "{refused:?}"
    );

    let reply: CastReply<CameraMoved> = crate::dev_client::post_json(
        port,
        path::CAMERA_ON,
        &CameraRequest {
            character: "elf".to_owned(),
            shot: None,
            over_ms: None,
        },
    )
    .await
    .unwrap();
    let DevReply::Rejected { error, .. } = reply else {
        panic!("a character the layout does not name is refused");
    };
    assert_eq!(
        error,
        CastError::Shown {
            cause: ShownSceneError::UnknownCharacter {
                character: "elf".to_owned(),
                picture: "second.png".to_owned(),
                names: vec!["dwarf".to_owned()],
            }
        }
    );
    assert_eq!(error.status(), axum::http::StatusCode::NOT_FOUND);

    // The scene path refuses this move and writes the one audit line; the cast route adds none.
    let before = running.state.log().after_sequence(0, usize::MAX).len();
    let reply: CastReply<CameraMoved> = crate::dev_client::post_json(
        port,
        path::CAMERA_ON,
        &CameraRequest {
            character: "dwarf".to_owned(),
            shot: None,
            over_ms: Some(u64::MAX),
        },
    )
    .await
    .unwrap();
    let DevReply::Rejected { error, .. } = reply else {
        panic!("a move longer than the longest allowed is refused");
    };
    assert!(
        matches!(
            error,
            CastError::Shown {
                cause: ShownSceneError::Scene { .. }
            }
        ),
        "{error:?}"
    );
    let written: Vec<String> = running
        .state
        .log()
        .after_sequence(0, usize::MAX)
        .into_iter()
        .skip(before)
        .map(|line| line.text)
        // The host edge's own `HTTP served` line of the request is not the playground's.
        .filter(|text| !text.starts_with("HTTP "))
        .collect();
    assert_eq!(
        written,
        vec!["PLAYGROUND reject cause=invalid_value reason=bad_camera_time".to_owned()]
    );
}

#[tokio::test]
async fn a_cast_request_without_a_shown_fixture_or_with_a_bad_body_is_a_typed_refusal() {
    use crate::test_support::{STARTER_FILES, start};
    let running = start(&STARTER_FILES).await;
    let port = running.port;
    let reply: CastReply<CastList> = crate::dev_client::get_json(port, path::CAST).await.unwrap();
    let DevReply::Rejected { error, .. } = reply else {
        panic!("no fixture is shown");
    };
    assert!(
        matches!(
            error,
            CastError::Shown {
                cause: ShownSceneError::Scene { .. }
            }
        ),
        "{error:?}"
    );
    assert_eq!(error.status(), axum::http::StatusCode::BAD_REQUEST);

    let reply: CastReply<Cued> =
        crate::dev_client::post_json(port, path::CUE, &serde_json::json!({"who": "elf"}))
            .await
            .unwrap();
    let DevReply::Rejected {
        error: CastError::InvalidBody(body),
        ..
    } = reply
    else {
        panic!("a body that is not a cue is refused");
    };
    assert_eq!(body.cause, crate::dev_http::BodyCause::Data);
    let refusals: Vec<String> = running
        .state
        .log()
        .after_sequence(0, usize::MAX)
        .into_iter()
        .map(|line| line.text)
        .filter(|text| text.starts_with("CAST refused"))
        .collect();
    assert_eq!(
        refusals,
        vec![
            "CAST refused verb=cast error=scene".to_owned(),
            "CAST refused verb=cue error=invalid_body".to_owned(),
        ]
    );
}

#[test]
fn every_cast_error_key_is_its_serde_tag_or_its_causes() {
    let body = crate::dev_http::parse_body::<CharacterCue>(b"{").unwrap_err();
    let invalid = CastError::InvalidBody(body);
    let written = serde_json::to_value(&invalid).unwrap();
    assert_eq!(written["error"], invalid.key(), "{written}");
    assert_eq!(
        serde_json::from_value::<CastError>(written).unwrap(),
        invalid
    );
    for cause in [
        ShownSceneError::NoBackdrop,
        ShownSceneError::NoLayout {
            picture: "a.png".to_owned(),
        },
        ShownSceneError::UnknownCharacter {
            character: "elf".to_owned(),
            picture: "a.png".to_owned(),
            names: Vec::new(),
        },
        ShownSceneError::Layout {
            cause: crate::locate::LocateError::Stopped,
        },
        ShownSceneError::Scene {
            cause: crate::playground::PlaygroundChangeError::InvalidChange {
                detail: "no fixture".to_owned(),
            },
        },
    ] {
        let tag = serde_json::to_value(&cause).unwrap()["error"].clone();
        assert_eq!(tag, cause.key());
        let error = CastError::Shown { cause };
        assert_eq!(error.key(), tag);
        let written = serde_json::to_value(&error).unwrap();
        assert_eq!(written["cause"]["error"], error.key(), "{written}");
        assert_eq!(serde_json::from_value::<CastError>(written).unwrap(), error);
    }
}
