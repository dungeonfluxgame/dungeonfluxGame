use df_scene_layout::{CharacterCue, CueKind};

use super::CastCommand;
use super::request::{CameraMoved, CameraRequest, CastError, CastList, CastReply, Cued};
use super::routes::path;
use crate::command::{Command, parse};
use crate::dev_http::{DevReply, RouteError};

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

/// A one-region layout naming `name`, bound to `picture` whose bytes are its own name.
fn layout_of(picture: &str, name: &str) -> df_scene_layout::SceneLayout {
    use df_scene_layout::{PictureBinding, RegionGeometry, SceneLayout, SceneRegion, region_id};
    let binding = PictureBinding {
        name: picture.to_owned(),
        sha256: df_imagery::oneshot_segment::picture_sha256(picture.as_bytes()),
        width_px: 100,
        height_px: 50,
    };
    let geometry = RegionGeometry {
        left_permille: 300,
        top_permille: 200,
        right_permille: 600,
        bottom_permille: 1000,
        head_x_permille: 450,
        head_y_permille: 200,
    };
    let region = SceneRegion::new(region_id(name.as_bytes()), geometry, 900)
        .unwrap()
        .with_label(Some(name.to_owned()));
    SceneLayout::new(binding, "sam-3.1", "person", vec![region])
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
    let port = running.port;
    let run = |line: &'static str| async move { crate::dev::run_dev(port, &words(line)).await };
    let first = run("scene cast").await.unwrap();
    assert!(first.contains("elf") && !first.contains("dwarf"), "{first}");

    run("scene image tv-backdrop second.png").await.unwrap();
    let second = run("scene cast").await.unwrap();
    assert!(
        second.contains("dwarf") && !second.contains("elf"),
        "{second}"
    );
    run("scene camera region:dwarf").await.unwrap();
    run("scene camera-on dwarf").await.unwrap();
    let refused = run("scene camera-on elf").await.unwrap_err();
    assert!(
        matches!(
            &refused,
            crate::dev::DevError::Rejected {
                error: crate::dev::ReplyError::Cast(CastError::UnknownCharacter { character, .. }),
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
        CastError::UnknownCharacter {
            character: "elf".to_owned(),
            picture: "second.png".to_owned(),
            names: vec!["dwarf".to_owned()],
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
    assert!(matches!(error, CastError::Scene { .. }), "{error:?}");
    let written: Vec<String> = running
        .state
        .log()
        .after_sequence(0, usize::MAX)
        .into_iter()
        .skip(before)
        .map(|line| line.text)
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
    assert!(matches!(error, CastError::Scene { .. }), "{error:?}");
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
fn every_cast_error_key_is_its_serde_tag() {
    let errors = [
        CastError::NoBackdrop,
        CastError::NoLayout {
            picture: "a.png".to_owned(),
        },
        CastError::UnknownCharacter {
            character: "elf".to_owned(),
            picture: "a.png".to_owned(),
            names: Vec::new(),
        },
        CastError::Layout {
            cause: crate::locate::LocateError::Stopped,
        },
        CastError::Scene {
            cause: crate::playground::PlaygroundChangeError::InvalidChange {
                detail: "no fixture".to_owned(),
            },
        },
    ];
    for error in errors {
        let written = serde_json::to_value(&error).unwrap();
        assert_eq!(written["error"], error.key(), "{written}");
        assert_eq!(serde_json::from_value::<CastError>(written).unwrap(), error);
    }
}
