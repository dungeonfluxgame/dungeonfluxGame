//! The camera: `scene camera` and `scene camera-reset`, parsed, checked against the stage, applied by
//! the host, and run back to back from a script with two avatars on the canvas.

use df_playground::fixture::Camera;
use df_playground::scene::{
    MAXIMUM_CAMERA_MS, MAXIMUM_ZOOM_PERCENT, MINIMUM_ZOOM_PERCENT, SceneChange,
};

use crate::client::get;
use crate::command::{
    Command, CommandError, DEFAULT_CAMERA_MS, DEFAULT_ZOOM_PERCENT, parse, split_words, usage_text,
};
use crate::dev::{DevError, run_dev};
use crate::playground::camera_problem;
use crate::test_support::{Running, STARTER_FILES, start};
use crate::wire::{ControlsReply, EventsReply};

const AVATAR_KIND: &str = include_str!("../../../assets/canvas/kinds/avatar.json");
const CAMERA_DEMO: &str = include_str!("../../../assets/playground/fixtures/camera_demo.json");
const TOUR: &str = include_str!("../../../development/playground-scripts/camera-tour.txt");

fn scene(line: &str) -> Result<SceneChange, CommandError> {
    let words = split_words(line).expect("the line splits");
    match parse(&words)? {
        Command::Scene(change) => Ok(change),
        other => panic!("expected a scene change, got {other:?}"),
    }
}

fn look(target: Option<&str>, zoom_percent: u16, duration_ms: u32) -> SceneChange {
    SceneChange::Camera {
        target: target.map(str::to_owned),
        zoom_percent,
        duration_ms,
    }
}

#[test]
fn the_camera_looks_at_an_element_twice_as_large_over_a_little_over_a_second_when_nothing_else_is_said()
 {
    assert_eq!(
        scene("scene camera finn"),
        Ok(look(Some("finn"), DEFAULT_ZOOM_PERCENT, DEFAULT_CAMERA_MS))
    );
    assert_eq!((DEFAULT_ZOOM_PERCENT, DEFAULT_CAMERA_MS), (200, 1_200));
}

#[test]
fn the_zoom_and_the_time_can_be_given_and_a_move_of_no_time_is_a_cut() {
    assert_eq!(
        scene("scene camera sigrun zoom=250 over=1500"),
        Ok(look(Some("sigrun"), 250, 1_500))
    );
    assert_eq!(
        scene("scene camera sigrun over=0"),
        Ok(look(Some("sigrun"), 200, 0))
    );
    assert_eq!(
        scene("scene camera sigrun zoom=100 over=30000"),
        Ok(look(Some("sigrun"), 100, 30_000))
    );
    assert_eq!(
        scene("scene camera sigrun zoom=400"),
        Ok(look(Some("sigrun"), 400, DEFAULT_CAMERA_MS))
    );
}

#[test]
fn reset_moves_the_camera_back_out_to_the_whole_stage() {
    assert_eq!(
        scene("scene camera-reset"),
        Ok(look(None, 100, DEFAULT_CAMERA_MS))
    );
    assert_eq!(
        scene("scene camera-reset over=500"),
        Ok(look(None, 100, 500))
    );
}

#[test]
fn a_zoom_or_time_out_of_range_or_a_key_that_does_not_belong_is_refused_with_the_range() {
    let too_far = format!("scene camera finn zoom={}", MAXIMUM_ZOOM_PERCENT + 1);
    let too_long = format!("scene camera finn over={}", MAXIMUM_CAMERA_MS + 1);
    for line in [
        "scene camera finn zoom=99",
        too_far.as_str(),
        "scene camera finn zoom=big",
        "scene camera finn zoom=-1",
        too_long.as_str(),
        "scene camera finn over=soon",
        "scene camera finn volume=3",
        "scene camera-reset zoom=200",
    ] {
        assert!(
            matches!(scene(line), Err(CommandError::BadChoice { .. })),
            "{line}"
        );
    }
    let Err(CommandError::BadChoice { allowed, .. }) = scene(&too_far) else {
        panic!("expected a refusal");
    };
    let range = format!("{MINIMUM_ZOOM_PERCENT} to {MAXIMUM_ZOOM_PERCENT}");
    assert!(allowed.contains(&range), "{allowed}");
    assert_eq!(
        scene("scene camera finn zoom=1 zoom=2"),
        Err(CommandError::DuplicateKey("zoom".into()))
    );
    assert!(matches!(scene("scene camera"), Err(CommandError::Usage(_))));
}

#[test]
fn the_help_describes_the_camera_and_what_stays_put() {
    let help = usage_text();
    for text in ["scene camera <element>", "scene camera-reset", "stay put"] {
        assert!(help.contains(text), "{text}");
    }
}

#[test]
fn a_camera_can_only_look_at_an_element_on_the_stage_and_the_refusal_names_the_ones_there() {
    let stage = ["finn".to_owned(), "sigrun".to_owned()];
    assert_eq!(camera_problem(&look(Some("finn"), 200, 0), &stage), None);
    assert_eq!(camera_problem(&look(None, 100, 0), &stage), None);
    assert_eq!(
        camera_problem(&SceneChange::Reset {}, &stage),
        None,
        "other changes are not the camera's business"
    );
    let message = camera_problem(&look(Some("nobody"), 200, 0), &stage).expect("refused");
    assert!(
        message.contains("no element 'nobody'") && message.contains("finn, sigrun"),
        "{message}"
    );
    let empty = camera_problem(&look(Some("nobody"), 200, 0), &[]).expect("refused");
    assert!(empty.contains("the stage is empty"), "{empty}");
}

async fn run(running: &Running, line: &str) -> Result<String, DevError> {
    run_dev(running.port, &split_words(line).expect("the line splits")).await
}

async fn shown(running: &Running) -> ControlsReply {
    let response = get(running.port, "/playground/controls.json")
        .await
        .expect("answers");
    serde_json::from_str(&response.body).expect("controls")
}

/// The camera as shown, without the host's reading of how far the move has run, which changes
/// with every look.
async fn camera_of(running: &Running) -> Option<Camera> {
    shown(running).await.scene.camera.map(|camera| Camera {
        elapsed_ms: 0,
        ..camera
    })
}

/// A host with the shipped avatar kind and the shipped camera scene chosen.
async fn with_the_camera_scene() -> Running {
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
    running
}

async fn with_two_avatars(running: &Running) {
    for line in [
        "spawn avatar finn x=30 y=30 name=Finn portrait=finn-alder.webp",
        "spawn avatar sigrun x=70 y=26 name=Sigrun portrait=sigrun.webp",
    ] {
        run(running, line).await.expect("spawns");
    }
}

#[tokio::test]
async fn the_camera_zooms_into_each_avatar_back_to_back_and_each_move_is_its_own_numbered_move() {
    let running = with_the_camera_scene().await;
    with_two_avatars(&running).await;
    assert_eq!(camera_of(&running).await, None, "the whole stage to begin");

    run(&running, "scene camera finn zoom=250 over=1500")
        .await
        .expect("zooms into Finn");
    let first = camera_of(&running).await.expect("a camera");
    assert_eq!(
        (
            first.target.as_deref(),
            first.zoom_percent,
            first.duration_ms
        ),
        (Some("finn"), 250, 1_500)
    );

    run(&running, "scene camera sigrun zoom=250 over=1500")
        .await
        .expect("zooms into Sigrun");
    let second = camera_of(&running).await.expect("a camera");
    assert_eq!(
        (second.target.as_deref(), second.zoom_percent),
        (Some("sigrun"), 250)
    );
    assert_eq!(
        (first.serial, second.serial),
        (1, 2),
        "a page tells a new move from one under way by its number"
    );

    run(&running, "scene camera-reset over=1500")
        .await
        .expect("pulls back");
    let last = camera_of(&running).await.expect("a camera");
    assert_eq!(
        (last.target, last.zoom_percent, last.serial),
        (None, 100, 3)
    );

    let events = get(running.port, "/events.json").await.expect("answers");
    let events: EventsReply = serde_json::from_str(&events.body).expect("events");
    let moves = events
        .events
        .iter()
        .filter(|line| line.text.starts_with("PLAYGROUND camera"))
        .count();
    assert_eq!(moves, 3, "each move is one audit line");
}

#[tokio::test]
async fn a_camera_aimed_at_something_that_is_not_there_is_refused_and_the_camera_stays_where_it_was()
 {
    let running = with_the_camera_scene().await;
    with_two_avatars(&running).await;
    run(&running, "scene camera finn")
        .await
        .expect("zooms into Finn");
    let before = camera_of(&running).await;
    let Err(error) = run(&running, "scene camera nobody").await else {
        panic!("a missing element must be refused");
    };
    let said = error.to_string();
    assert!(
        said.contains("no element 'nobody'") && said.contains("finn, sigrun"),
        "{said}"
    );
    assert_eq!(camera_of(&running).await, before);
}

#[tokio::test]
async fn a_zoom_out_of_range_that_slips_past_the_command_line_is_refused_by_the_host() {
    let running = with_the_camera_scene().await;
    with_two_avatars(&running).await;
    let body = serde_json::json!({
        "change": "camera", "target": "finn", "zoom_percent": MAXIMUM_ZOOM_PERCENT + 1, "duration_ms": 10
    });
    let response =
        crate::client::post_json(running.port, "/dev/playground/scene", &body.to_string())
            .await
            .expect("answers");
    assert_eq!(response.status, 400);
    let range = format!("{MINIMUM_ZOOM_PERCENT} to {MAXIMUM_ZOOM_PERCENT}");
    assert!(response.body.contains(&range), "{}", response.body);
    assert_eq!(camera_of(&running).await, None);
}

#[tokio::test]
async fn resetting_the_scene_brings_the_camera_back_to_the_file_but_keeps_counting_moves() {
    let running = with_the_camera_scene().await;
    with_two_avatars(&running).await;
    run(&running, "scene camera finn").await.expect("zooms");
    run(&running, "scene reset").await.expect("resets");
    assert_eq!(camera_of(&running).await, None);
    run(&running, "scene camera sigrun").await.expect("zooms");
    assert_eq!(
        camera_of(&running).await.expect("a camera").serial,
        2,
        "the count only rises, so a page never mistakes a new move for an old one"
    );
}

#[tokio::test]
async fn the_shipped_tour_script_runs_and_ends_on_the_whole_stage_after_both_zooms() {
    let running = with_the_camera_scene().await;
    // The pauses are real time in the shipped script; the check runs the same lines without them.
    let quick: String = TOUR
        .lines()
        .filter(|line| !line.starts_with("wait "))
        .map(|line| format!("{line}\n"))
        .collect();
    let directory = tempfile::TempDir::new().expect("makes a directory");
    let path = directory.path().join("tour.txt");
    std::fs::write(&path, quick).expect("writes the script");
    let words = ["script".to_owned(), path.to_string_lossy().into_owned()];
    let said = run_dev(running.port, &words).await.expect("runs");
    assert!(said.ends_with("all ok"), "{said}");
    let canvas = get(running.port, "/canvas.json").await.expect("answers");
    for avatar in ["finn", "sigrun"] {
        assert!(
            canvas.body.contains(&format!("\"id\":\"{avatar}\"")),
            "{avatar}"
        );
    }
    assert!(canvas.body.contains("finn-alder.webp") && canvas.body.contains("sigrun.webp"));
    let last = camera_of(&running).await.expect("a camera");
    assert_eq!(
        (last.target, last.zoom_percent, last.serial),
        (None, 100, 3)
    );
}
