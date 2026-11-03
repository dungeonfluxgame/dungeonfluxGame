//! The Director against a running host: a verb moves the playground's camera through the same
//! path as the dev camera verbs, with the same numbers, and the lines it writes are the lines
//! `director show` prints.

use df_playground::fixture::Camera;

use crate::client::{get, post_json};
use crate::command::split_words;
use crate::dev::{DevError, run_dev};
use crate::test_support::{Running, STARTER_FILES, start};
use crate::wire::{ControlsReply, EventsReply};

const AVATAR_KIND: &str = include_str!("../../../../assets/canvas/kinds/avatar.json");
pub(super) const CAMERA_DEMO: &str =
    include_str!("../../../../assets/playground/fixtures/camera_demo.json");
const TOUR: &str = include_str!("../../../../development/playground-scripts/director-tour.txt");

pub(super) async fn run(running: &Running, line: &str) -> Result<String, DevError> {
    run_dev(running.port, &split_words(line).expect("the line splits")).await
}

pub(super) async fn said(running: &Running, line: &str) -> String {
    run(running, line)
        .await
        .unwrap_or_else(|error| panic!("'{line}' failed: {error}"))
}

async fn shown(running: &Running) -> ControlsReply {
    let response = get(running.port, "/playground/controls.json")
        .await
        .expect("answers");
    serde_json::from_str(&response.body).expect("controls")
}

pub(super) async fn camera_of(running: &Running) -> Option<Camera> {
    shown(running).await.scene.camera.map(|camera| Camera {
        elapsed_ms: 0,
        ..camera
    })
}

pub(super) async fn lines_starting(running: &Running, prefix: &str) -> Vec<String> {
    let response = get(running.port, "/events.json").await.expect("answers");
    let events: EventsReply = serde_json::from_str(&response.body).expect("events");
    events
        .events
        .into_iter()
        .map(|line| line.text)
        .filter(|text| text.starts_with(prefix))
        .collect()
}

pub(super) async fn with_three_avatars() -> Running {
    let running = start(&[
        STARTER_FILES[0],
        STARTER_FILES[1],
        ("avatar.json", AVATAR_KIND),
    ])
    .await;
    running.write_asset("fixtures/camera_demo.json", CAMERA_DEMO);
    get(running.port, "/playground/controls.json")
        .await
        .expect("answers");
    for line in [
        "spawn avatar finn x=30 y=30 name=Finn portrait=finn-alder.webp",
        "spawn avatar sigrun x=70 y=26 name=Sigrun portrait=sigrun.webp",
        "spawn avatar lurker x=50 y=40 name=Lurker",
    ] {
        said(&running, line).await;
    }
    running
}

#[tokio::test]
async fn camera_focus_moves_the_camera_with_the_dev_verbs_numbers_and_writes_a_reasoned_line() {
    let running = with_three_avatars().await;
    let answer = said(&running, "camera focus finn reason=speaker").await;
    assert!(
        answer.contains(
            "CAMERA focus element=finn zoom=200 over=1200 reason=speaker serial=1 driven=true"
        ),
        "{answer}"
    );
    let camera = camera_of(&running).await.expect("a camera");
    assert_eq!(
        (
            camera.target.as_deref(),
            camera.zoom_percent,
            camera.duration_ms,
            camera.serial
        ),
        (Some("finn"), 200, 1_200, 1)
    );
    assert_eq!(
        lines_starting(&running, "CAMERA focus").await,
        ["CAMERA focus element=finn zoom=200 over=1200 reason=speaker serial=1 driven=true"]
    );
    assert_eq!(
        lines_starting(&running, "PLAYGROUND camera").await.len(),
        1,
        "the playground's own audit line is written by the same change"
    );
}

#[tokio::test]
async fn a_second_focus_inside_the_spacing_waits_then_pans_on_without_pulling_out() {
    let running = with_three_avatars().await;
    said(&running, "camera focus finn reason=speaker").await;
    let answer = said(&running, "camera focus sigrun reason=speaker").await;
    assert!(answer.contains("CAMERA queued"), "{answer}");
    assert_eq!(
        camera_of(&running)
            .await
            .expect("a camera")
            .target
            .as_deref(),
        Some("finn")
    );
    said(&running, "director advance 800").await;
    let camera = camera_of(&running).await.expect("a camera");
    assert_eq!(
        (camera.target.as_deref(), camera.serial),
        (Some("sigrun"), 2)
    );
    assert!(lines_starting(&running, "CAMERA wide").await.is_empty());
}

#[tokio::test]
async fn a_creature_that_becomes_hidden_is_pulled_back_and_cannot_be_focused_while_hidden() {
    let running = with_three_avatars().await;
    said(&running, "camera focus lurker reason=speaker").await;
    assert!(camera_of(&running).await.is_some());
    said(&running, "director view hide lurker").await;
    assert_eq!(camera_of(&running).await.expect("a camera").target, None);
    assert_eq!(
        lines_starting(&running, "CAMERA wide").await,
        ["CAMERA wide reason=target_hidden serial=2 over=1200"]
    );
    said(&running, "director advance 1000").await;
    let answer = said(&running, "camera focus lurker reason=speaker").await;
    assert!(
        answer.contains("CAMERA dropped reason=target_hidden direction=speaker element=lurker"),
        "{answer}"
    );
    assert_eq!(camera_of(&running).await.expect("a camera").target, None);
}

#[tokio::test]
async fn a_name_that_is_not_on_the_stage_and_a_number_out_of_range_are_refused() {
    let running = with_three_avatars().await;
    let answer = said(&running, "camera focus nobody").await;
    assert!(answer.contains("reason=unknown_target"), "{answer}");
    assert_eq!(camera_of(&running).await, None);
    for (line, expected) in [
        ("camera focus finn zoom=99", "100 to 10000"),
        ("camera focus finn over=100", "600 to 2500"),
        ("camera focus finn hold=0", "a hold is 1 to"),
        ("camera focus Finn", "not an element name"),
    ] {
        let Err(error) = run(&running, line).await else {
            panic!("'{line}' must be refused");
        };
        assert!(error.to_string().contains(expected), "{line}: {error}");
    }
    let response = post_json(running.port, "/dev/director/act", r#"{"verb":"dance"}"#)
        .await
        .expect("answers");
    assert_eq!(response.status, 400);
    assert!(response.body.contains("bad_request"), "{}", response.body);
    assert_eq!(camera_of(&running).await, None);
}

#[tokio::test]
async fn director_show_lists_what_was_queued_applied_and_dropped_and_matches_the_log() {
    let running = with_three_avatars().await;
    said(&running, "director view hide lurker").await;
    said(&running, "camera focus finn reason=speaker").await;
    said(&running, "camera focus lurker reason=hit").await;
    said(&running, "camera focus sigrun reason=requested").await;
    let text = said(&running, "director show").await;
    assert!(text.contains("holding: finn for speaker"), "{text}");
    assert!(text.contains("hidden lurker"), "{text}");
    assert!(
        text.contains("directions: 1 queued, 1 applied, 2 dropped"),
        "{text}"
    );
    let written = lines_starting(&running, "CAMERA").await;
    assert!(!written.is_empty());
    for line in &written {
        assert!(
            text.contains(line.as_str()),
            "{line} is in the log and not in show:\n{text}"
        );
    }
}

#[tokio::test]
async fn the_shipped_director_tour_runs_and_ends_with_the_camera_pulled_back() {
    let running = with_three_avatars().await;
    let directory = tempfile::TempDir::new().expect("makes a directory");
    let path = directory.path().join("tour.txt");
    std::fs::write(&path, TOUR).expect("writes the script");
    let words = ["script".to_owned(), path.to_string_lossy().into_owned()];
    let answer = run_dev(running.port, &words).await.expect("runs");
    assert!(answer.ends_with("all ok"), "{answer}");
    assert_eq!(camera_of(&running).await.expect("a camera").target, None);
    let focus = lines_starting(&running, "CAMERA focus").await;
    let wide = lines_starting(&running, "CAMERA wide").await;
    assert!(focus.len() >= 5 && wide.len() >= 3, "{focus:?} {wide:?}");
    assert!(
        wide.iter()
            .any(|line| line.contains("reason=target_hidden")),
        "{wide:?}"
    );
    assert!(
        lines_starting(&running, "CAMERA dropped reason=target_hidden")
            .await
            .iter()
            .any(|line| line.contains("element=lurker")),
        "the hidden lurker was refused"
    );
}
