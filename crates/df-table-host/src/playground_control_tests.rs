//! The playground's controls and fixture commands, through the running host.

use df_playground::controls::{ControlsChange, Device};
use serde_json::json;

use crate::client::{get, post_json};
use crate::playground::fixture_file_name;
use crate::test_support::{Running, STARTER_FILES, post_request, raw_exchange, start};
use crate::wire::{ControlsReply, EventsReply, Reply};

const MINIMAL_FIXTURE: &str = r#"{
    "name": "One",
    "tv": {"place": "Here", "status": {"kind": "ok"}, "elements": []},
    "phone": {"hero": "Pip", "status": {"kind": "ok"}}
}"#;

async fn controls(running: &Running) -> ControlsReply {
    let response = get(running.port, "/playground/controls.json")
        .await
        .expect("answers");
    assert_eq!(response.status, 200);
    serde_json::from_str(&response.body).expect("controls")
}

async fn change(running: &Running, change: &ControlsChange) -> (u16, String) {
    let response = post_json(
        running.port,
        "/dev/playground/controls",
        &serde_json::to_string(change).expect("writes"),
    )
    .await
    .expect("answers");
    (response.status, response.body)
}

async fn events(running: &Running) -> Vec<String> {
    let response = get(running.port, "/events.json").await.expect("answers");
    let reply: EventsReply = serde_json::from_str(&response.body).expect("events");
    reply.events.into_iter().map(|line| line.text).collect()
}

#[test]
fn a_fixture_name_may_leave_off_its_extension_but_not_leave_its_directory() {
    assert_eq!(fixture_file_name("tavern"), Ok("tavern.json".to_owned()));
    assert_eq!(
        fixture_file_name("tavern.json"),
        Ok("tavern.json".to_owned())
    );
    for bad in [
        "",
        ".json",
        "../x",
        "a/b",
        "a b",
        ".hidden",
        "a\\b",
        &"a".repeat(80),
    ] {
        assert!(fixture_file_name(bad).is_err(), "{bad:?}");
    }
}

#[tokio::test]
async fn with_no_fixture_the_controls_start_empty_and_the_first_is_chosen_once_one_exists() {
    let running = start(&STARTER_FILES).await;
    assert_eq!(controls(&running).await.controls.fixture, None);
    running.write_asset("fixtures/b.json", MINIMAL_FIXTURE);
    running.write_asset("fixtures/a.json", MINIMAL_FIXTURE);
    let reply = controls(&running).await;
    assert_eq!(reply.controls.fixture.as_deref(), Some("a.json"));
    assert_eq!(reply.revision, 1);
    assert_eq!(
        controls(&running).await.revision,
        1,
        "reading does not change it"
    );
}

#[tokio::test]
async fn a_change_is_applied_numbered_and_recorded() {
    let running = start(&STARTER_FILES).await;
    running.write_asset("fixtures/a.json", MINIMAL_FIXTURE);
    controls(&running).await;
    let (status, body) = change(&running, &ControlsChange::Device(Device::Tv)).await;
    assert_eq!(status, 200);
    let reply: ControlsReply = serde_json::from_str(&body).expect("controls");
    assert_eq!(reply.controls.device, Device::Tv);
    assert_eq!(controls(&running).await, reply);
    assert!(
        events(&running)
            .await
            .iter()
            .any(|line| line == "PLAYGROUND set device=tv revision=2")
    );
}

#[tokio::test]
async fn setting_what_is_already_set_does_not_count_as_a_change() {
    let running = start(&STARTER_FILES).await;
    let (_, first) = change(&running, &ControlsChange::ShowGrid(true)).await;
    let (_, second) = change(&running, &ControlsChange::ShowGrid(true)).await;
    assert_eq!(first, second);
}

#[tokio::test]
async fn a_refused_change_is_a_400_that_says_why_and_leaves_the_controls_alone() {
    let running = start(&STARTER_FILES).await;
    running.write_asset("fixtures/a.json", MINIMAL_FIXTURE);
    let before = controls(&running).await;
    let (status, body) = change(&running, &ControlsChange::Status("sleepy".into())).await;
    assert_eq!(status, 400);
    assert!(body.contains("waiting_for"));
    let (status, body) = change(&running, &ControlsChange::Fixture("ghost.json".into())).await;
    assert_eq!(status, 400);
    assert!(body.contains("a.json"), "{body}");
    assert_eq!(controls(&running).await, before);
    assert!(
        events(&running)
            .await
            .iter()
            .any(|line| line.starts_with("PLAYGROUND reject cause=invalid_value"))
    );
}

#[tokio::test]
async fn a_body_that_is_not_a_change_is_a_400_and_recorded() {
    let running = start(&STARTER_FILES).await;
    for body in ["", "nope", r#"{"change":"volume","value":3}"#] {
        let response = post_json(running.port, "/dev/playground/controls", body)
            .await
            .expect("answers");
        assert_eq!(response.status, 400, "{body}");
    }
    let count = events(&running)
        .await
        .iter()
        .filter(|line| line.contains("PLAYGROUND reject cause=malformed_request"))
        .count();
    assert_eq!(count, 3);
}

#[tokio::test]
async fn a_removed_fixture_that_was_chosen_is_replaced_by_the_first_left() {
    let running = start(&STARTER_FILES).await;
    running.write_asset("fixtures/a.json", MINIMAL_FIXTURE);
    running.write_asset("fixtures/b.json", MINIMAL_FIXTURE);
    change(&running, &ControlsChange::Fixture("b.json".into())).await;
    std::fs::remove_file(running.playground_path("assets/fixtures/b.json")).expect("removes");
    assert_eq!(
        controls(&running).await.controls.fixture.as_deref(),
        Some("a.json")
    );
}

#[tokio::test]
async fn a_fixture_that_reads_is_written_listed_shown_and_removed() {
    let running = start(&STARTER_FILES).await;
    let body = json!({"name": "one", "text": MINIMAL_FIXTURE}).to_string();
    let response = post_json(running.port, "/dev/playground/fixture", &body)
        .await
        .expect("answers");
    assert_eq!(response.status, 200);
    assert!(matches!(
        serde_json::from_str::<Reply>(&response.body),
        Ok(Reply::Done { .. })
    ));
    let list = get(running.port, "/playground/fixtures.json")
        .await
        .expect("answers");
    assert_eq!(list.body, r#"["one.json"]"#);
    let shown = get(running.port, "/playground/fixtures/one.json")
        .await
        .expect("answers");
    assert_eq!(shown.body, MINIMAL_FIXTURE);
    assert!(
        events(&running)
            .await
            .iter()
            .any(|line| line.starts_with("PLAYGROUND fixture_write name=one.json"))
    );

    let removed = post_json(
        running.port,
        "/dev/playground/fixture/remove",
        r#"{"name":"one"}"#,
    )
    .await
    .expect("answers");
    assert_eq!(removed.status, 200);
    assert_eq!(
        get(running.port, "/playground/fixtures.json")
            .await
            .expect("answers")
            .body,
        "[]"
    );
    let again = post_json(
        running.port,
        "/dev/playground/fixture/remove",
        r#"{"name":"one"}"#,
    )
    .await
    .expect("answers");
    assert_eq!(again.status, 400);
    assert!(again.body.contains("no fixture"));
}

#[tokio::test]
async fn a_fixture_that_does_not_read_is_refused_with_its_position_and_nothing_is_written() {
    let running = start(&STARTER_FILES).await;
    let text = MINIMAL_FIXTURE.replace("\"hero\"", "\"heroo\"");
    let body = json!({"name": "bad", "text": text}).to_string();
    let response = post_json(running.port, "/dev/playground/fixture", &body)
        .await
        .expect("answers");
    assert_eq!(response.status, 400);
    assert!(
        response.body.contains("bad.json, line"),
        "{}",
        response.body
    );
    assert_eq!(
        get(running.port, "/playground/fixtures.json")
            .await
            .expect("answers")
            .body,
        "[]"
    );
}

#[tokio::test]
async fn a_fixture_name_that_leaves_the_directory_is_refused_before_anything_is_written() {
    let running = start(&STARTER_FILES).await;
    for name in ["../escape", "a/b", ""] {
        let body = json!({"name": name, "text": MINIMAL_FIXTURE}).to_string();
        let response = post_json(running.port, "/dev/playground/fixture", &body)
            .await
            .expect("answers");
        assert_eq!(response.status, 400, "{name:?}");
    }
    assert!(!running.playground_path("escape.json").exists());
}

#[tokio::test]
async fn the_playground_commands_are_behind_the_same_guard_as_the_canvas_commands() {
    let running = start(&STARTER_FILES).await;
    let request = post_request(
        "/dev/playground/controls",
        r#"{"change":"show_grid","value":true}"#,
        "Origin: http://evil.example\r\n",
    );
    assert!(
        raw_exchange(running.port, &request)
            .await
            .starts_with("HTTP/1.1 403")
    );
    assert!(!controls(&running).await.controls.show_grid);
}
