use serde_json::json;

use crate::client::{get, post_json};
use crate::server::{MAXIMUM_EVENTS_PER_ANSWER, MAXIMUM_REQUEST_BYTES};
use crate::test_support::{Running, STARTER_FILES, post_request, raw_exchange, start};
use crate::view::CanvasView;
use crate::wire::{EventsReply, Reply};

async fn post(running: &Running, body: serde_json::Value) -> (u16, Reply) {
    let response = post_json(running.port, "/dev/effect", &body.to_string())
        .await
        .expect("answers");
    let reply = serde_json::from_str(&response.body).expect("the answer is a reply");
    (response.status, reply)
}

async fn events(running: &Running, query: &str) -> EventsReply {
    let response = get(running.port, &format!("/events.json{query}"))
        .await
        .expect("answers");
    assert_eq!(response.status, 200);
    serde_json::from_str(&response.body).expect("the answer is an events reply")
}

async fn canvas(running: &Running) -> CanvasView {
    let response = get(running.port, "/canvas.json").await.expect("answers");
    serde_json::from_str(&response.body).expect("the answer is a canvas view")
}

fn texts(reply: &EventsReply) -> Vec<&str> {
    reply.events.iter().map(|line| line.text.as_str()).collect()
}

fn spawn_marker(id: &str) -> serde_json::Value {
    json!({"op": "spawn", "id": id, "kind": "marker"})
}

#[tokio::test]
async fn the_viewer_page_is_served_with_the_canvas_in_it() {
    let running = start(&STARTER_FILES).await;
    post(&running, spawn_marker("m1")).await;
    let page = get(running.port, "/").await.expect("answers");
    assert_eq!(page.status, 200);
    assert!(page.body.contains("Blank canvas") && page.body.contains("m1"));
}

#[tokio::test]
async fn a_spawn_is_answered_with_the_new_revision_and_shows_in_the_canvas() {
    let running = start(&STARTER_FILES).await;
    let (status, reply) = post(&running, spawn_marker("m1")).await;
    assert_eq!(status, 200);
    assert_eq!(
        reply,
        Reply::Done {
            revision: 1,
            notices: Vec::new()
        }
    );
    assert_eq!(canvas(&running).await.elements.len(), 1);
}

#[tokio::test]
async fn a_refused_change_is_a_400_with_its_cause_and_is_recorded_with_effect_and_element() {
    let running = start(&STARTER_FILES).await;
    let (status, reply) = post(
        &running,
        json!({"op": "spawn", "id": "x", "kind": "dragon"}),
    )
    .await;
    assert_eq!(status, 400);
    assert!(matches!(reply, Reply::Rejected { cause: Some(_), .. }));
    let log = events(&running, "").await;
    assert!(
        texts(&log)
            .iter()
            .any(|line| line.contains("reject effect=spawn cause=unknown_kind"))
    );
    let (_, _) = post(&running, json!({"op": "despawn", "id": "ghost"})).await;
    let log = events(&running, "").await;
    assert!(
        texts(&log)
            .iter()
            .any(|line| line.contains("effect=despawn") && line.contains("element=ghost"))
    );
}

#[tokio::test]
async fn a_body_that_is_not_an_effect_is_a_400_and_is_recorded() {
    let running = start(&STARTER_FILES).await;
    for body in [
        "not json",
        r#"{"op": "dance"}"#,
        r#"{"op": "clear", "extra": 1}"#,
        "",
    ] {
        let response = post_json(running.port, "/dev/effect", body)
            .await
            .expect("answers");
        assert_eq!(response.status, 400, "{body}");
    }
    let log = events(&running, "").await;
    assert_eq!(
        texts(&log)
            .iter()
            .filter(|line| line.contains("cause=malformed_request"))
            .count(),
        4
    );
}

#[tokio::test]
async fn a_request_over_the_size_limit_is_refused_and_recorded() {
    let running = start(&STARTER_FILES).await;
    let body = format!(
        r#"{{"op": "spawn", "id": "a", "kind": "marker", "fields": {{"note": "{}"}}}}"#,
        "x".repeat(MAXIMUM_REQUEST_BYTES)
    );
    let request = post_request("/dev/effect", &body, "");
    let answer = raw_exchange(running.port, &request).await;
    assert!(
        answer.starts_with("HTTP/1.1 413"),
        "{}",
        answer.lines().next().unwrap_or("")
    );
    let log = events(&running, "").await;
    assert!(
        texts(&log)
            .iter()
            .any(|line| line.contains("cause=malformed_request"))
    );
}

#[tokio::test]
async fn a_foreign_host_is_refused_even_for_a_read_and_the_refusal_is_recorded() {
    let running = start(&STARTER_FILES).await;
    let answer = raw_exchange(
        running.port,
        "GET /canvas.json HTTP/1.1\r\nHost: evil.example\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert!(answer.starts_with("HTTP/1.1 403"));
    assert!(
        texts(&events(&running, "").await)
            .iter()
            .any(|line| line.contains("cause=forbidden_request"))
    );
}

#[tokio::test]
async fn a_form_post_to_reload_is_refused_so_a_web_page_cannot_trigger_it() {
    let running = start(&STARTER_FILES).await;
    running.write_file(
        "wall.json",
        r#"{"name": "wall", "shape": "rect", "width": 10, "height": 2}"#,
    );
    let form = "POST /dev/reload HTTP/1.1\r\nHost: 127.0.0.1\r\n\
                Content-Type: application/x-www-form-urlencoded\r\nContent-Length: 0\r\n\
                Connection: close\r\n\r\n";
    assert!(
        raw_exchange(running.port, form)
            .await
            .starts_with("HTTP/1.1 415")
    );
    assert_eq!(
        canvas(&running).await.kinds.len(),
        2,
        "the kinds were not reloaded"
    );
}

#[tokio::test]
async fn a_post_from_another_origin_is_refused() {
    let running = start(&STARTER_FILES).await;
    let request = post_request(
        "/dev/effect",
        &spawn_marker("m1").to_string(),
        "Origin: http://evil.example\r\n",
    );
    assert!(
        raw_exchange(running.port, &request)
            .await
            .starts_with("HTTP/1.1 403")
    );
    assert!(canvas(&running).await.elements.is_empty());
}

#[tokio::test]
async fn events_come_oldest_first_and_after_a_sequence_only_the_newer_ones() {
    let running = start(&STARTER_FILES).await;
    for id in ["a", "b", "c"] {
        post(&running, spawn_marker(id)).await;
    }
    let all = events(&running, "").await;
    assert_eq!(all.latest, 3);
    assert!(texts(&all)[0].contains("element=a") && texts(&all)[2].contains("element=c"));
    let newer = events(&running, "?after=2").await;
    assert_eq!(newer.events.len(), 1);
    assert!(texts(&newer)[0].contains("element=c"));
}

#[tokio::test]
async fn without_a_sequence_the_newest_page_is_returned_not_the_oldest() {
    let running = start(&STARTER_FILES).await;
    let total = MAXIMUM_EVENTS_PER_ANSWER + 5;
    for index in 0..total {
        post(&running, spawn_marker(&format!("m{index}"))).await;
        post(
            &running,
            json!({"op": "despawn", "id": format!("m{index}")}),
        )
        .await;
    }
    let page = events(&running, "").await;
    assert_eq!(page.events.len(), MAXIMUM_EVENTS_PER_ANSWER);
    assert_eq!(page.events.last().expect("has lines").sequence, page.latest);
}

#[tokio::test]
async fn a_reader_ahead_of_the_newest_sequence_gets_the_newest_page_after_a_restart() {
    let running = start(&STARTER_FILES).await;
    post(&running, spawn_marker("a")).await;
    let reply = events(&running, "?after=500").await;
    assert_eq!(reply.latest, 1);
    assert_eq!(reply.events.len(), 1);
}

#[tokio::test]
async fn a_reload_reports_the_new_revision_and_records_it() {
    let running = start(&STARTER_FILES).await;
    running.write_file(
        "wall.json",
        r#"{"name": "wall", "shape": "rect", "width": 10, "height": 2}"#,
    );
    let response = post_json(running.port, "/dev/reload", "")
        .await
        .expect("answers");
    assert_eq!(response.status, 200);
    assert_eq!(canvas(&running).await.kinds.len(), 3);
    assert!(
        texts(&events(&running, "").await)
            .iter()
            .any(|line| line.contains("reload kinds=3"))
    );
}

#[tokio::test]
async fn a_reload_that_leaves_an_element_without_a_fitting_kind_says_so_in_a_notice() {
    let running = start(&STARTER_FILES).await;
    post(&running, spawn_marker("m1")).await;
    std::fs::remove_file(running.kind_directory().join("marker.json")).expect("removes the file");
    let response = post_json(running.port, "/dev/reload", "")
        .await
        .expect("answers");
    let Reply::Done { notices, .. } = serde_json::from_str(&response.body).expect("a reply") else {
        panic!("expected the reload to be accepted");
    };
    assert!(
        notices.iter().any(|notice| notice.contains("m1")),
        "{notices:?}"
    );
    assert!(canvas(&running).await.elements[0].problem.is_some());
}

#[tokio::test]
async fn a_reload_with_a_broken_file_is_refused_and_the_old_kinds_stay() {
    let running = start(&STARTER_FILES).await;
    running.write_file("broken.json", "{ not json");
    let response = post_json(running.port, "/dev/reload", "")
        .await
        .expect("answers");
    assert_eq!(response.status, 400);
    let Reply::Rejected { problems, .. } = serde_json::from_str(&response.body).expect("a reply")
    else {
        panic!("expected a rejection");
    };
    assert!(
        problems
            .iter()
            .any(|problem| problem.contains("broken.json"))
    );
    assert_eq!(canvas(&running).await.kinds.len(), 2);
    assert!(
        texts(&events(&running, "").await)
            .iter()
            .any(|line| line.contains("reload_rejected errors=1"))
    );
}

#[tokio::test]
async fn an_unknown_route_is_a_404() {
    let running = start(&STARTER_FILES).await;
    assert_eq!(
        get(running.port, "/nowhere").await.expect("answers").status,
        404
    );
}
