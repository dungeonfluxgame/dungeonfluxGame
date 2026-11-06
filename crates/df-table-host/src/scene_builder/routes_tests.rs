//! The scene builder's dev verbs through a running host: the routes, the guard, the command line
//! and a script, on an ephemeral port.

use crate::command::split_words;
use crate::dev::run_dev;
use crate::dev_http::{BodyCause, BodyError};
use crate::test_support::{STARTER_FILES, post_request, raw_exchange, start};
use crate::wire::EventsReply;

use super::fixtures::refusal_of;
use super::request::{BuildReply, SceneBuilderError, SceneReply};
use super::routes::path;

async fn dev(port: u16, line: &str) -> String {
    let words = split_words(line).expect("the line splits");
    run_dev(port, &words)
        .await
        .unwrap_or_else(|error| panic!("'{line}' failed: {error}"))
}

#[tokio::test]
async fn scene_build_from_the_command_line_builds_the_dev_scene_and_numbers_its_answer() {
    let host = start(&STARTER_FILES).await;

    let answer = dev(host.port, "scene build \"Harbor market\"").await;
    let mut lines = answer.lines();
    let head = lines.next().expect("a head line");
    assert!(
        head.starts_with("scene 1 (generation ") && head.contains("Harbor market"),
        "{head}"
    );
    assert!(
        head.contains("the first plate was on screen after"),
        "{head}"
    );
    assert!(
        lines.next().expect("a first line").starts_with("1: "),
        "{answer}"
    );
    assert!(answer.contains("SCENE stage_stored"), "{answer}");
    assert!(answer.contains("PLATE swapped"), "{answer}");

    let shown = dev(host.port, "scene show").await;
    assert!(
        shown.contains("description stored, background stored, first plate ready"),
        "{shown}"
    );
    assert!(shown.contains("person local: unlocated"), "{shown}");
    assert!(shown.contains("item urn: own_picture"), "{shown}");

    let plates = dev(host.port, "plates").await;
    assert!(
        plates.contains("interval 15 s: at most 240 plates an hour"),
        "{plates}"
    );
    let buffer = dev(host.port, "buffer show").await;
    assert!(buffer.contains("on screen: plate of job 1"), "{buffer}");
}

#[tokio::test]
async fn the_clock_runs_from_the_command_line_and_a_quiet_table_pays_for_nothing() {
    let host = start(&STARTER_FILES).await;
    dev(host.port, "scene build quay").await;
    let busy = dev(host.port, "plates run 60").await;
    assert!(busy.contains("PLATE tick_queued"), "{busy}");
    assert!(busy.contains("PLATE swapped"), "{busy}");
    let quiet = dev(host.port, "plates run 45 quiet").await;
    assert!(quiet.contains("reason=nothing_changed"), "{quiet}");
    assert!(!quiet.contains("PLATE tick_queued"), "{quiet}");
}

#[tokio::test]
async fn the_events_of_a_build_reach_the_hosts_event_log() {
    let host = start(&STARTER_FILES).await;
    dev(host.port, "scene build quay").await;
    let response = crate::client::get(host.port, crate::wire::route::EVENTS)
        .await
        .expect("events");
    let events: EventsReply = serde_json::from_str(&response.body).expect("events parse");
    assert!(
        events
            .events
            .iter()
            .any(|line| line.text.starts_with("SCENE stage_stored")),
        "the scene builder's lines are in the log"
    );
    assert!(
        events
            .events
            .iter()
            .any(|line| line.text.starts_with("RESOURCE ready"))
    );
}

#[tokio::test]
async fn a_request_before_any_build_and_a_bad_body_are_typed_refusals() {
    let host = start(&STARTER_FILES).await;
    let early = raw_exchange(
        host.port,
        &post_request(path::PLATES, r#"{"seconds":10}"#, ""),
    )
    .await;
    assert!(early.starts_with("HTTP/1.1 400"), "{early}");
    assert_eq!(refusal_of(&early), SceneBuilderError::NothingBuilt);
    run_dev(host.port, &split_words("plates run 10").expect("splits"))
        .await
        .expect_err("the command line fails too");

    let bad = raw_exchange(
        host.port,
        &post_request(path::BUILD, r#"{"place":"quay","surprise":1}"#, ""),
    )
    .await;
    assert!(bad.starts_with("HTTP/1.1 400"), "{bad}");
    assert!(
        matches!(
            refusal_of(&bad),
            SceneBuilderError::InvalidBody(BodyError {
                cause: BodyCause::Data,
                ..
            })
        ),
        "{bad}"
    );

    let blank = raw_exchange(
        host.port,
        &post_request(path::BUILD, r#"{"place":" "}"#, ""),
    )
    .await;
    assert!(blank.starts_with("HTTP/1.1 400"), "{blank}");
    assert!(
        matches!(refusal_of(&blank), SceneBuilderError::InvalidBody(_)),
        "a place that is not a name is not a body of scene build: {blank}"
    );
    let refusals = host
        .state
        .log()
        .newest(10_000)
        .into_iter()
        .filter(|line| line.text == "SCENE refused verb=build error=invalid_body")
        .count();
    assert_eq!(refusals, 2, "one refusal line for each refused body");
}

#[tokio::test]
async fn the_routes_are_behind_the_hosts_guard() {
    let host = start(&STARTER_FILES).await;
    let foreign = raw_exchange(
        host.port,
        &format!(
            "GET {} HTTP/1.1\r\nHost: example.com\r\nConnection: close\r\n\r\n",
            path::SHOW
        ),
    )
    .await;
    assert!(foreign.starts_with("HTTP/1.1 403"), "{foreign}");

    let not_json = raw_exchange(
        host.port,
        &format!(
            "POST {} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: text/plain\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}",
            path::BUILD
        ),
    )
    .await;
    assert!(not_json.starts_with("HTTP/1.1 415"), "{not_json}");

    let cross_site = raw_exchange(
        host.port,
        &post_request(
            path::BUILD,
            r#"{"place":"quay"}"#,
            "Origin: http://example.com\r\n",
        ),
    )
    .await;
    assert!(cross_site.starts_with("HTTP/1.1 403"), "{cross_site}");
    let shown = dev(host.port, "scene show").await;
    assert!(
        shown.contains("no scene is open"),
        "nothing the guard refused built a scene: {shown}"
    );
}

#[tokio::test]
async fn a_reply_reads_back_as_the_typed_value() {
    let host = start(&STARTER_FILES).await;
    let response = raw_exchange(
        host.port,
        &post_request(path::BUILD, r#"{"place":"quay"}"#, ""),
    )
    .await;
    let body = response.split("\r\n\r\n").nth(1).expect("a body");
    let reply: SceneReply<BuildReply> = serde_json::from_str(body).expect("a typed reply");
    let SceneReply::Done { value } = reply else {
        panic!("the build was refused: {body}");
    };
    assert_eq!(value.place.as_str(), "quay");
    assert!(!value.lines.is_empty());
}
