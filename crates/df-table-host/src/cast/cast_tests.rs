use df_scene_layout::{CharacterCue, CueKind};

use super::CastCommand;
use super::request::{CameraMoved, CameraRequest, CastError, CastList, CastReply, Cued};
use super::routes::path;
use super::routes::{SpeakerMove, speaker_move};
use crate::command::{Command, parse};
use crate::dev_http::{DevReply, RouteError};
use crate::shown_scene::ShownSceneError;
use crate::test_support::{layout_of, layout_of_cast};

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
    assert!(
        first.contains("elf Sylwen (Scout of the pass) ["),
        "{first}"
    );
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

fn push(character: &str) -> SpeakerMove {
    SpeakerMove::Push {
        target: format!("region:{character}:speaker"),
        zoom_percent: 112,
    }
}

#[test]
fn a_speaker_with_a_region_gets_the_gentle_push_in_and_the_next_speaker_takes_it() {
    let layout = layout_of_cast("plate.png", &["elf", "dwarf"]);
    assert_eq!(speaker_move(Some(&layout), "elf", None), push("elf"));
    assert_eq!(
        speaker_move(Some(&layout), "dwarf", Some("region:elf:speaker")),
        push("dwarf")
    );
    assert_eq!(
        speaker_move(Some(&layout), "elf", Some("region:dwarf")),
        push("elf")
    );
}

#[test]
fn the_same_speaker_again_holds_still() {
    let layout = layout_of_cast("plate.png", &["elf", "dwarf"]);
    for held in ["region:elf:speaker", "region:elf", "region:elf:face"] {
        assert_eq!(
            speaker_move(Some(&layout), "elf", Some(held)),
            SpeakerMove::Hold,
            "{held}"
        );
    }
}

#[test]
fn a_speaker_without_a_region_leaves_the_camera_wide() {
    let layout = layout_of_cast("plate.png", &["elf"]);
    assert_eq!(
        speaker_move(Some(&layout), "bard", Some("region:elf:speaker")),
        SpeakerMove::Wide
    );
    assert_eq!(
        speaker_move(None, "elf", Some("region:elf:speaker")),
        SpeakerMove::Wide
    );
    assert_eq!(speaker_move(None, "elf", None), SpeakerMove::Hold);
    assert_eq!(speaker_move(Some(&layout), "bard", None), SpeakerMove::Hold);
}

#[test]
fn a_camera_held_on_something_else_is_left_alone() {
    let layout = layout_of_cast("plate.png", &["elf"]);
    assert_eq!(
        speaker_move(Some(&layout), "elf", Some("finn")),
        SpeakerMove::Hold
    );
    assert_eq!(speaker_move(None, "elf", Some("finn")), SpeakerMove::Hold);
}

#[tokio::test]
async fn a_spoken_cue_moves_the_camera_and_a_thinking_cue_does_not() {
    use crate::test_support::{STARTER_FILES, start};
    let running = start(&STARTER_FILES).await;
    running.write_asset("media/plate.png", "plate.png");
    crate::locate::write_layout(
        &running.playground_path("assets/layouts"),
        "plate.png",
        &layout_of_cast("plate.png", &["elf", "dwarf"]),
    )
    .unwrap();
    running.write_asset(
        "fixtures/one.json",
        concat!(
            r#"{"name": "One", "tv": {"place": "Here", "status": {"kind": "ok"}, "elements": [], "#,
            r#""backdrop": {"media": "plate.png", "caption": null}}, "#,
            r#""phone": {"hero": "Pip", "status": {"kind": "ok"}}}"#
        ),
    );
    let port = running.port;
    let run = |line: &'static str| async move { crate::dev::run_dev(port, &words(line)).await };
    let serial = || {
        running
            .state
            .playground()
            .snapshot()
            .scene
            .camera
            .set_value()
            .map(|camera| camera.serial)
    };
    run("scene cast").await.unwrap();

    run("scene cue elf think").await.unwrap();
    assert_eq!(running.state.playground().camera_target(), None);

    run("scene cue elf speak Well met.").await.unwrap();
    assert_eq!(
        running.state.playground().camera_target().as_deref(),
        Some("region:elf:speaker")
    );
    let pushed = serial();

    run("scene cue elf speak Again.").await.unwrap();
    assert_eq!(serial(), pushed, "the same speaker holds still");

    run("scene cue dwarf think").await.unwrap();
    assert_eq!(serial(), pushed, "a thinking cue leaves the camera");

    run("scene cue dwarf speak Aye.").await.unwrap();
    assert_eq!(
        running.state.playground().camera_target().as_deref(),
        Some("region:dwarf:speaker")
    );
    let log: Vec<String> = running
        .state
        .log()
        .after_sequence(0, usize::MAX)
        .into_iter()
        .map(|line| line.text)
        .filter(|text| text.starts_with("CAMERA speaker"))
        .collect();
    assert_eq!(log.len(), 3, "{log:?}");
    assert!(
        log[0].contains("character=elf move=push target=region:elf:speaker zoom=112 over=2000")
    );
    assert!(log[1].contains("character=elf move=hold"));
    assert!(log[2].contains("character=dwarf move=push"));
}

/// A host showing a plate with `elf` and `dwarf` on the clock `start_paused` stops.
async fn paused_host() -> crate::test_support::Running {
    use crate::test_support::{STARTER_FILES, start};
    let running = start(&STARTER_FILES).await;
    running.write_asset("media/plate.png", "plate.png");
    crate::locate::write_layout(
        &running.playground_path("assets/layouts"),
        "plate.png",
        &layout_of_cast("plate.png", &["elf", "dwarf"]),
    )
    .unwrap();
    running.write_asset(
        "fixtures/one.json",
        concat!(
            r#"{"name": "One", "tv": {"place": "Here", "status": {"kind": "ok"}, "elements": [], "#,
            r#""backdrop": {"media": "plate.png", "caption": null}}, "#,
            r#""phone": {"hero": "Pip", "status": {"kind": "ok"}}}"#
        ),
    );
    crate::dev::run_dev(running.port, &words("scene cast"))
        .await
        .unwrap();
    running
}

/// Moves the paused clock on by `ms` and lets the pull-back task, and the blocking reads it waits
/// for, finish.
async fn pass(ms: u64) {
    tokio::time::advance(std::time::Duration::from_millis(ms)).await;
    for _ in 0..200 {
        tokio::task::yield_now().await;
        std::thread::yield_now();
    }
}

fn rests(running: &crate::test_support::Running) -> usize {
    running
        .state
        .log()
        .after_sequence(0, usize::MAX)
        .into_iter()
        .filter(|line| line.text.starts_with("CAMERA speaker") && line.text.contains("move=rest"))
        .count()
}

#[tokio::test(start_paused = true)]
async fn a_new_line_cancels_the_pending_pull_back() {
    use super::routes::SPEAKER_REST_MS;
    let running = paused_host().await;
    let port = running.port;
    let run = |line: &'static str| async move { crate::dev::run_dev(port, &words(line)).await };
    run("scene cue elf speak Well.").await.unwrap();
    pass(SPEAKER_REST_MS - 2_000).await;
    run("scene cue elf speak Again.").await.unwrap();
    // Past the first line's deadline, before the second's.
    pass(SPEAKER_REST_MS - 2_000).await;
    assert_eq!(rests(&running), 0);
    assert_eq!(
        running.state.playground().camera_target().as_deref(),
        Some("region:elf:speaker")
    );
    pass(3_000).await;
    assert_eq!(rests(&running), 1, "only the second line's timer ran");
}

#[tokio::test(start_paused = true)]
async fn the_pull_back_quiets_the_last_speaker_and_sends_the_camera_wide() {
    use super::routes::SPEAKER_REST_MS;
    let running = paused_host().await;
    let port = running.port;
    crate::dev::run_dev(port, &words("scene cue elf speak Well."))
        .await
        .unwrap();
    let bubbles = || {
        running
            .state
            .playground()
            .snapshot()
            .scene
            .roleplay
            .set_value()
            .map(|card| card.bubbles.len())
    };
    assert_eq!(bubbles(), Some(1));
    pass(SPEAKER_REST_MS + 1_000).await;
    assert_eq!(rests(&running), 1);
    assert_eq!(running.state.playground().camera_target(), None);
    assert_eq!(bubbles().unwrap_or(0), 0, "the line left with the camera");
}
