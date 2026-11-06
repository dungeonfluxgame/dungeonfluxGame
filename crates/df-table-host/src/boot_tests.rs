//! Which run of the host an answer is from, and how long ago a move began, so a page that keeps
//! running across a host restart or opens mid-move can carry on correctly.

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
    controls(running)
        .await
        .scene
        .camera
        .set_value()
        .cloned()
        .expect("a camera")
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

#[test]
fn run_names_have_one_width_and_sort_in_the_order_the_runs_started() {
    // Built the way `AppState` builds one: the page orders runs by comparing the names.
    let names = [
        crate::state::new_boot_id(),
        crate::state::new_boot_id(),
        crate::state::new_boot_id(),
    ];
    assert!(names.iter().all(|name| name.len() == names[0].len()));
    assert!(names.windows(2).all(|pair| pair[0] < pair[1]), "{names:?}");
}

/// A move reads as having begun at the host's clock, with the time since in `elapsed_ms`. The clock
/// is the host's own and is not set here, so the test holds what does not depend on how long it
/// takes: the start is stamped once and never moves, the age never goes down between reads, and
/// how an age is made from a start is tested without a clock in `continuity_tests`.
#[tokio::test]
async fn a_move_is_stamped_with_the_hosts_clock_once_and_every_read_says_how_old_it_is() {
    let running = with_finn().await;
    run(&running, "scene camera finn over=60000")
        .await
        .expect("moves");
    let first = camera_of(&running).await;
    let second = camera_of(&running).await;
    assert_eq!(second.serial, first.serial);
    assert_eq!(second.began_ms, first.began_ms, "the start does not move");
    assert!(
        second.elapsed_ms >= first.elapsed_ms,
        "{first:?} then {second:?}"
    );
    let in_frame = frame(&running)
        .await
        .controls
        .scene
        .camera
        .set_value()
        .cloned()
        .expect("the frame carries the move too");
    assert_eq!(
        (in_frame.serial, in_frame.began_ms),
        (first.serial, first.began_ms)
    );
    assert!(in_frame.elapsed_ms >= second.elapsed_ms);
}

#[tokio::test]
async fn a_new_move_is_the_next_serial_and_begins_no_earlier_than_the_last() {
    let running = with_finn().await;
    run(&running, "scene camera finn over=60000")
        .await
        .expect("moves");
    let first = camera_of(&running).await;
    run(&running, "scene camera finn over=60000")
        .await
        .expect("moves again");
    let again = camera_of(&running).await;
    assert_eq!(again.serial, first.serial + 1);
    assert!(again.began_ms >= first.began_ms);
}

#[tokio::test]
async fn a_slide_is_stamped_the_same_way() {
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
        df_playground::overlay::Edit::Set(image) => image.slide.expect("a slide"),
        other => panic!("a card with a slide, got {other:?}"),
    };
    let first = slide(controls(&running).await);
    let later = slide(controls(&running).await);
    assert_eq!(later.serial, first.serial);
    assert_eq!(later.began_ms, first.began_ms);
    assert!(later.elapsed_ms >= first.elapsed_ms);
}

/// A page that kept a frame from before a restart names the old run; a restarted host whose count
/// has reached the page's must not hold that request until its next change.
#[tokio::test]
async fn a_frame_request_naming_another_run_is_answered_at_once_even_at_the_same_count() {
    let running = start(&STARTER_FILES).await;
    let count = running.state.change_count();
    let patience = std::time::Duration::from_millis(60);
    let this_run = controls(&running).await.boot_id;
    let held = crate::server::FrameQuery::for_test(Some(count), Some(204)).with_boot(&this_run);
    let answer = crate::server::frame_answer_for_test(&running.state, &held, patience).await;
    assert_eq!(answer.status().as_u16(), 204, "this run, nothing changed");
    let other_run = crate::server::FrameQuery::for_test(Some(count), Some(204)).with_boot("0-old");
    let answer = crate::server::frame_answer_for_test(&running.state, &other_run, patience).await;
    assert_eq!(answer.status().as_u16(), 200, "another run gets the frame");
}

#[tokio::test]
async fn a_frame_request_reads_the_query_the_page_writes_and_refuses_an_unknown_parameter() {
    let running = start(&STARTER_FILES).await;
    let boot_id = controls(&running).await.boot_id;
    let address = df_playground::frame::frame_address(
        df_playground::path::FRAME,
        Some(running.state.change_count() + 1),
        Some(&boot_id),
    );
    assert_eq!(
        get(running.port, &address).await.expect("answers").status,
        200
    );
    let unknown = format!("{}?after=0&bot=x", df_playground::path::FRAME);
    assert_eq!(
        get(running.port, &unknown).await.expect("answers").status,
        400
    );
}
