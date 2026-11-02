//! Which run of the host an answer is from, and how long ago a move began, so a page that keeps
//! running across a host restart or opens mid-move can carry on correctly.

use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use df_playground::fixture::Camera;
use serde_json::json;

use crate::client::{get, post_json};
use crate::command::split_words;
use crate::dev::{DevError, run_dev};
use crate::test_support::{Running, STARTER_FILES, start};
use crate::wire::{ControlsReply, Frame};

const AVATAR_KIND: &str = include_str!("../../../assets/canvas/kinds/avatar.json");
const CAMERA_DEMO: &str = include_str!("../../../assets/playground/fixtures/camera_demo.json");

async fn run(running: &Running, line: &str) -> Result<String, DevError> {
    run_dev(running.port, &split_words(line).expect("the line splits")).await
}

async fn controls(running: &Running) -> ControlsReply {
    let response = get(running.port, "/playground/controls.json")
        .await
        .expect("answers");
    serde_json::from_str(&response.body).expect("controls")
}

async fn frame(running: &Running) -> Frame {
    let response = get(running.port, "/playground/frame")
        .await
        .expect("answers");
    serde_json::from_str(&response.body).expect("a frame")
}

async fn with_finn() -> Running {
    let running = start(&[
        STARTER_FILES[0],
        STARTER_FILES[1],
        ("avatar.json", AVATAR_KIND),
    ])
    .await;
    running.write_asset("fixtures/camera_demo.json", CAMERA_DEMO);
    controls(&running).await;
    run(&running, "spawn avatar finn x=30 y=30 name=Finn")
        .await
        .expect("spawns");
    running
}

async fn camera_of(running: &Running) -> Camera {
    controls(running).await.scene.camera.expect("a camera")
}

#[tokio::test]
async fn the_controls_and_the_frame_name_the_run_of_the_host_and_a_restart_has_another_name() {
    let first = start(&STARTER_FILES).await;
    let second = start(&STARTER_FILES).await;
    let named = controls(&first).await.boot_id;
    assert!(!named.is_empty(), "a run has a name");
    assert_eq!(controls(&first).await.boot_id, named, "set once at startup");
    assert_eq!(
        frame(&first).await.controls.boot_id,
        named,
        "the frame says the same"
    );
    run(&first, "playground set grid=true")
        .await
        .expect("changes");
    assert_eq!(
        controls(&first).await.boot_id,
        named,
        "a change does not rename the run"
    );
    assert_ne!(
        controls(&second).await.boot_id,
        named,
        "another start, another name"
    );
}

#[tokio::test]
async fn a_move_says_how_long_ago_it_began_by_the_hosts_clock_and_a_new_move_begins_again() {
    let running = with_finn().await;
    run(&running, "scene camera finn over=60000")
        .await
        .expect("moves");
    let at_once = camera_of(&running).await;
    assert!(at_once.elapsed_ms < 1_000, "{at_once:?}");
    tokio::time::sleep(Duration::from_millis(250)).await;
    let later = camera_of(&running).await;
    assert_eq!(later.serial, at_once.serial);
    assert_eq!(later.began_ms, at_once.began_ms, "the start does not move");
    assert!(
        later.elapsed_ms >= at_once.elapsed_ms + 200,
        "{at_once:?} then {later:?}"
    );
    assert_eq!(
        frame(&running)
            .await
            .controls
            .scene
            .camera
            .map(|camera| camera.serial),
        Some(later.serial),
        "the frame carries the move too"
    );
    run(&running, "scene camera finn over=60000")
        .await
        .expect("moves again");
    let again = camera_of(&running).await;
    assert_eq!(again.serial, later.serial + 1);
    assert!(again.began_ms >= later.began_ms + 200);
    assert!(again.elapsed_ms < later.elapsed_ms);
}

#[tokio::test]
async fn a_slide_says_how_long_ago_it_began_too() {
    let running = with_finn().await;
    let body = json!({"name": "map.png", "data_base64": STANDARD.encode(b"not really a picture")})
        .to_string();
    let added = post_json(running.port, "/dev/playground/media", &body)
        .await
        .expect("answers");
    assert_eq!(added.status, 200, "{}", added.body);
    run(&running, "scene slide tv-card map.png over=60000")
        .await
        .expect("slides");
    let slide = |reply: ControlsReply| match reply.scene.card {
        Some(df_playground::scene::Override::Set(image)) => image.slide.expect("a slide"),
        other => panic!("a card with a slide, got {other:?}"),
    };
    let first = slide(controls(&running).await);
    tokio::time::sleep(Duration::from_millis(250)).await;
    let later = slide(controls(&running).await);
    assert_eq!(later.serial, first.serial);
    assert!(
        later.elapsed_ms >= first.elapsed_ms + 200,
        "{first:?} then {later:?}"
    );
}
