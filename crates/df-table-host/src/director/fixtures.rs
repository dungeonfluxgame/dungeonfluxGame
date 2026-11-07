//! Shared fixtures for the Director's tests against a running host: a host with three avatars on
//! the camera demo, a command line run against it, and what the playground and the log show.

use df_playground::fixture::Camera;
use df_playground::path;

use crate::client::get;
use crate::command::split_words;
use crate::dev::{DevError, run_dev};
use crate::test_support::{Running, STARTER_FILES, start};
use crate::wire::{ControlsReply, EventsReply};

const AVATAR_KIND: &str = include_str!("../../../../assets/canvas/kinds/avatar.json");
pub(super) const CAMERA_DEMO: &str =
    include_str!("../../../../assets/playground/fixtures/camera_demo.json");

pub(super) async fn run(running: &Running, line: &str) -> Result<String, DevError> {
    run_dev(running.port, &split_words(line).expect("the line splits")).await
}

pub(super) async fn said(running: &Running, line: &str) -> String {
    run(running, line)
        .await
        .unwrap_or_else(|error| panic!("'{line}' failed: {error}"))
}

async fn shown(running: &Running) -> ControlsReply {
    let response = get(running.port, path::CONTROLS).await.expect("answers");
    serde_json::from_str(&response.body).expect("controls")
}

pub(super) async fn camera_of(running: &Running) -> Option<Camera> {
    shown(running)
        .await
        .scene
        .camera
        .set_value()
        .map(|camera| Camera {
            elapsed_ms: 0,
            ..camera.clone()
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
    get(running.port, path::CONTROLS).await.expect("answers");
    for line in [
        "spawn avatar finn x=30 y=30 name=Finn portrait=finn-alder.webp",
        "spawn avatar sigrun x=70 y=26 name=Sigrun portrait=sigrun.webp",
        "spawn avatar lurker x=50 y=40 name=Lurker",
    ] {
        said(&running, line).await;
    }
    running
}
