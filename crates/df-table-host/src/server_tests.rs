use axum::http::StatusCode;
use serde_json::json;

use crate::client::{get, post_json};
use crate::limits::{MAXIMUM_EVENTS_PER_ANSWER, MAXIMUM_REQUEST_BYTES};
use crate::test_support::{Running, STARTER_FILES, post_request, raw_exchange, start};
use crate::view::CanvasView;
use crate::wire::{EventsReply, Reply};

async fn post(running: &Running, body: serde_json::Value) -> (StatusCode, Reply) {
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
    assert_eq!(response.status, StatusCode::OK);
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
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body.contains("Blank canvas") && page.body.contains("m1"));
}

#[tokio::test]
async fn a_spawn_is_answered_with_the_new_revision_and_shows_in_the_canvas() {
    let running = start(&STARTER_FILES).await;
    let (status, reply) = post(&running, spawn_marker("m1")).await;
    assert_eq!(status, StatusCode::OK);
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
    assert_eq!(status, StatusCode::BAD_REQUEST);
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
        assert_eq!(response.status, StatusCode::BAD_REQUEST, "{body}");
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
    // Each change is two lines: the canvas's and the `HTTP served` line of its request. (The
    // host's background services write their own start lines meanwhile, so lines are counted by
    // area.)
    let served = events(&running, "?area=HTTP").await;
    assert_eq!(served.events.len(), 3, "{served:?}");
    let canvas = events(&running, "?area=CANVAS").await;
    assert_eq!(canvas.events.len(), 3);
    assert!(texts(&canvas)[0].contains("element=a") && texts(&canvas)[2].contains("element=c"));
    let second = canvas.events[1].sequence;
    let newer = events(&running, &format!("?after={second}&area=CANVAS")).await;
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
    // Every line so far: the spawn's, its request's `HTTP served` line, and any start lines of
    // the host's background services.
    assert_eq!(
        u64::try_from(reply.events.len()).expect("fits"),
        reply.latest
    );
    assert!(texts(&reply).iter().any(|line| line.contains("element=a")));
    assert!(
        texts(&reply)
            .iter()
            .any(|line| line.starts_with("HTTP served method=post route=/dev/effect "))
    );
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
    assert_eq!(response.status, StatusCode::OK);
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
    assert_eq!(response.status, StatusCode::BAD_REQUEST);
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
        StatusCode::NOT_FOUND
    );
}

/// A canvas with marker `m1` travelling two units at a hundred units a second.
fn state_with_a_travelling_marker() -> (tempfile::TempDir, std::sync::Arc<crate::state::AppState>) {
    use df_table::testkit::{MARKER_KIND, registry_from};
    let directory = tempfile::TempDir::new().expect("makes a directory");
    let log = std::sync::Arc::new(crate::event_log::EventLog::default());
    let state = std::sync::Arc::new(crate::state::AppState::new(
        df_table::canvas::Canvas::new(registry_from(&[MARKER_KIND])),
        crate::observe::HostEventSink::new(log, std::sync::Arc::new(crate::observe::WallClock)),
        None,
        directory.path().to_path_buf(),
        crate::playground::PlaygroundFiles::new(
            directory.path().join("assets"),
            directory.path().join("pkg"),
        ),
    ));
    for request in [
        json!({"op": "spawn", "kind": "marker", "id": "m1", "x": 10.0, "y": 10.0}),
        json!({"op": "travel", "id": "m1", "x": 12.0, "y": 10.0, "speed": 100.0}),
    ] {
        let request: crate::wire::EffectRequest =
            serde_json::from_value(request).expect("an effect");
        let mut canvas = state.canvas();
        let effect = request.into_effect(&canvas).expect("converts");
        canvas.apply(effect, state.sink()).expect("applies");
    }
    (directory, state)
}

#[tokio::test(start_paused = true)]
async fn the_clock_moves_travelling_elements_by_tokio_time_so_paused_time_drives_it() {
    let (_directory, state) = state_with_a_travelling_marker();
    assert!(state.is_moving());
    let clock = crate::server::spawn_clock(std::sync::Arc::clone(&state));
    // A tenth of a second of tokio's clock covers three ticks; at a hundred units a second the
    // two units are crossed on the first full tick. The real time that passes is almost none.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    clock.abort();
    assert!(state.tick_count() > 0);
    assert!(!state.is_moving(), "the marker arrived");
}

/// The texts of every line the host has logged, oldest first.
fn logged(running: &Running) -> Vec<String> {
    running
        .state
        .log()
        .newest(MAXIMUM_EVENTS_PER_ANSWER)
        .into_iter()
        .map(|line| line.text)
        .collect()
}

fn lines_starting(lines: &[String], prefix: &str) -> Vec<String> {
    lines
        .iter()
        .filter(|line| line.starts_with(prefix))
        .cloned()
        .collect()
}

/// The `duration_ms` value a line ends with, checked to be a number.
fn ends_with_a_duration(line: &str) -> bool {
    line.rsplit_once(" duration_ms=")
        .is_some_and(|(_, value)| value.parse::<u64>().is_ok())
}

#[tokio::test]
async fn a_change_on_a_dev_route_writes_one_http_served_line_with_its_route_template() {
    let running = start(&STARTER_FILES).await;
    post(&running, spawn_marker("m1")).await;
    let served = lines_starting(&logged(&running), "HTTP served ");
    assert_eq!(served.len(), 1, "{served:?}");
    assert!(
        served[0].starts_with("HTTP served method=post route=/dev/effect status=200 duration_ms="),
        "{served:?}"
    );
    assert!(ends_with_a_duration(&served[0]), "{served:?}");
}

#[tokio::test]
async fn a_request_too_large_for_the_controls_route_is_served_413_and_recorded_with_its_reason() {
    let running = start(&STARTER_FILES).await;
    let body = format!(
        r#"{{"change": "status", "value": "{}"}}"#,
        "x".repeat(MAXIMUM_REQUEST_BYTES)
    );
    let request = post_request("/dev/playground/controls", &body, "");
    let answer = raw_exchange(running.port, &request).await;
    assert!(
        answer.starts_with("HTTP/1.1 413"),
        "{}",
        answer.lines().next().unwrap_or("")
    );
    let lines = logged(&running);
    let served = lines_starting(&lines, "HTTP served ");
    assert_eq!(served.len(), 1, "{lines:?}");
    assert!(
        served[0].starts_with("HTTP served method=post route=/dev/playground/controls status=413 "),
        "{served:?}"
    );
    assert!(
        lines.contains(
            &"PLAYGROUND reject cause=malformed_request reason=too_large source=- client=-"
                .to_owned()
        ),
        "{lines:?}"
    );
}

#[tokio::test]
async fn a_successful_dev_read_and_the_frame_poll_write_no_http_served_line() {
    let running = start(&STARTER_FILES).await;
    let status = get(running.port, crate::server::path::HOST_STATUS)
        .await
        .expect("answers");
    assert_eq!(status.status, StatusCode::OK);
    let frame = get(running.port, "/playground/frame")
        .await
        .expect("answers");
    assert_eq!(frame.status, StatusCode::OK);
    let lines = logged(&running);
    assert!(
        lines_starting(&lines, "HTTP ").is_empty(),
        "a read that succeeded is not a served line: {lines:?}"
    );
}

#[tokio::test]
async fn an_events_query_that_does_not_read_is_refused_typed_and_recorded() {
    let running = start(&STARTER_FILES).await;
    for query in ["?after=x", "?colour=red", "?area=lower", "?limit=-1"] {
        let response = get(running.port, &format!("/events.json{query}"))
            .await
            .expect("answers");
        assert_eq!(response.status, StatusCode::BAD_REQUEST, "{query}");
        let reply: crate::server::request::HostReply<()> =
            serde_json::from_str(&response.body).expect("a host reply");
        assert!(
            matches!(
                reply,
                crate::dev_http::DevReply::Rejected {
                    error: crate::server::request::HostError::InvalidQuery { .. },
                    ..
                }
            ),
            "{query}: {reply:?}"
        );
    }
    let refused = lines_starting(&logged(&running), "HOST refused ");
    assert_eq!(
        refused,
        vec!["HOST refused route=events error=invalid_query".to_owned(); 4]
    );
}

#[tokio::test]
async fn events_filter_by_area_and_limit_and_carry_their_time_and_the_forgotten_count() {
    let running = start(&STARTER_FILES).await;
    for id in ["a", "b", "c"] {
        post(&running, spawn_marker(id)).await;
    }
    let canvas = events(&running, "?area=CANVAS&limit=2").await;
    assert_eq!(canvas.events.len(), 2, "{canvas:?}");
    assert!(
        canvas
            .events
            .iter()
            .all(|line| line.text.starts_with("CANVAS "))
    );
    assert!(texts(&canvas)[1].contains("element=c"), "the newest two");
    assert!(canvas.events.iter().all(|line| line.at_ms > 0));
    assert_eq!(canvas.forgotten, 0);
    let first = events(&running, "?after=0&area=HTTP&limit=1").await;
    assert_eq!(first.events.len(), 1);
    assert!(texts(&first)[0].starts_with("HTTP served "));
}

#[tokio::test]
async fn host_status_answers_the_log_the_sink_and_the_clock() {
    let running = start(&STARTER_FILES).await;
    post(&running, spawn_marker("m1")).await;
    let response = get(running.port, crate::server::path::HOST_STATUS)
        .await
        .expect("answers");
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);
    let reply: crate::server::request::HostReply<crate::server::request::HostStatus> =
        serde_json::from_str(&response.body).expect("a host reply");
    let crate::dev_http::DevReply::Done { value: status } = reply else {
        panic!("not done: {}", response.body);
    };
    assert_eq!(status.log.latest, running.state.log().latest_sequence());
    // Nothing was forgotten, so every line is kept: at least the spawn and its served line.
    assert_eq!(
        u64::try_from(status.log.kept).expect("fits"),
        status.log.latest
    );
    assert!(status.log.latest >= 2, "{:?}", status.log);
    assert_eq!(status.log.forgotten, 0);
    assert!(!status.log.echo_writer_gone);
    assert_eq!(status.sink.always, 1, "{:?}", status.sink);
    assert_eq!(status.clock.moving, 0);
    assert_eq!(status.clock.tick_ms, 33);
    assert!(status.engine_sink.is_none());
}

/// Reads the frame as a page does (naming itself in `X-Df-Client`) and gives the status line.
async fn page_reads_frame(running: &Running, query: &str) -> String {
    let request = format!(
        "GET /playground/frame{query} HTTP/1.1\r\nHost: 127.0.0.1\r\n\
         X-Df-Client: page-1a2b3c4d\r\nConnection: close\r\n\r\n"
    );
    let answer = raw_exchange(running.port, &request).await;
    answer.lines().next().unwrap_or("").to_owned()
}

#[tokio::test]
async fn a_frame_is_recorded_once_for_each_change_however_often_pages_read_it() {
    let running = start(&STARTER_FILES).await;
    for _ in 0..2 {
        let status = page_reads_frame(&running, "").await;
        assert!(status.starts_with("HTTP/1.1 200"), "{status}");
    }
    post(&running, spawn_marker("m1")).await;
    let status = page_reads_frame(&running, "?after=0").await;
    assert!(status.starts_with("HTTP/1.1 200"), "{status}");
    let served = lines_starting(&logged(&running), "FRAME served ");
    assert_eq!(served.len(), 2, "{served:?}");
    assert!(
        served[0].starts_with("FRAME served frame=0 after=- bytes="),
        "{served:?}"
    );
    assert!(
        served[1].starts_with("FRAME served frame=1 after=0 bytes="),
        "{served:?}"
    );
    assert!(
        served.iter().all(|line| line.contains(" waited_ms=")
            && line.ends_with(" source=page client=page-1a2b3c4d"))
    );
}

#[tokio::test]
async fn the_command_lines_frame_reads_write_no_frame_line_and_leave_it_to_the_page() {
    let running = start(&STARTER_FILES).await;
    post(&running, spawn_marker("m1")).await;
    for query in ["", "?after=0"] {
        let frame = get(running.port, &format!("/playground/frame{query}"))
            .await
            .expect("answers");
        assert_eq!(frame.status, StatusCode::OK);
    }
    assert_eq!(
        lines_starting(&logged(&running), "FRAME served "),
        Vec::<String>::new()
    );
    let status = page_reads_frame(&running, "").await;
    assert!(status.starts_with("HTTP/1.1 200"), "{status}");
    let served = lines_starting(&logged(&running), "FRAME served ");
    assert_eq!(served.len(), 1, "{served:?}");
    assert!(
        served[0].starts_with("FRAME served frame=1 after=- "),
        "{served:?}"
    );
}

#[tokio::test]
async fn the_clocks_steps_of_a_travel_write_no_frame_line() {
    let running = start(&STARTER_FILES).await;
    for line in ["spawn marker m1 x=10 y=10", "travel m1 20 10 speed=10"] {
        let words = crate::command::split_words(line).expect("splits");
        crate::dev::run_dev(running.port, &words)
            .await
            .expect("runs");
    }
    let status = page_reads_frame(&running, "").await;
    assert!(status.starts_with("HTTP/1.1 200"), "{status}");
    let travelling = running.state.change_count();
    for step in 1..=3 {
        running.state.tick(0.25);
        let status = page_reads_frame(&running, &format!("?after={}", travelling + step - 1)).await;
        assert!(status.starts_with("HTTP/1.1 200"), "{status}");
    }
    assert_eq!(
        running.state.change_count(),
        travelling + 3,
        "each step woke the pages"
    );
    assert_eq!(
        lines_starting(&logged(&running), "FRAME served ").len(),
        1,
        "only the change before the travel"
    );
    post(&running, spawn_marker("m2")).await;
    let status = page_reads_frame(&running, "").await;
    assert!(status.starts_with("HTTP/1.1 200"), "{status}");
    let served = lines_starting(&logged(&running), "FRAME served ");
    assert_eq!(served.len(), 2, "{served:?}");
    assert!(
        served[1].starts_with(&format!("FRAME served frame={} ", travelling + 4)),
        "{served:?}"
    );
}

#[tokio::test]
async fn a_frame_query_that_does_not_read_is_refused_and_recorded() {
    let running = start(&STARTER_FILES).await;
    let response = get(running.port, "/playground/frame?after=soon")
        .await
        .expect("answers");
    assert_eq!(response.status, StatusCode::BAD_REQUEST);
    assert!(
        response.body.contains(r#""error":"invalid_query""#),
        "{}",
        response.body
    );
    assert_eq!(
        lines_starting(&logged(&running), "HOST refused "),
        vec!["HOST refused route=frame error=invalid_query".to_owned()]
    );
}

#[tokio::test]
async fn a_forbidden_request_names_its_method_and_no_raw_path() {
    let running = start(&STARTER_FILES).await;
    let answer = raw_exchange(
        running.port,
        "GET /playground/fixtures/secret-name.json HTTP/1.1\r\nHost: evil.example\r\n\
         Connection: close\r\n\r\n",
    )
    .await;
    assert!(answer.starts_with("HTTP/1.1 403"));
    let refused = lines_starting(&logged(&running), "HTTP refused ");
    assert_eq!(
        refused,
        vec!["HTTP refused method=get route=unmatched cause=forbidden_request".to_owned()]
    );
}

#[tokio::test(start_paused = true)]
async fn the_clock_records_a_stall_its_idle_its_wake_and_its_stop() {
    let (_directory, state) = state_with_a_travelling_marker();
    let lines = |state: &crate::state::AppState| -> Vec<String> {
        state
            .log()
            .newest(100)
            .into_iter()
            .map(|line| line.text)
            .filter(|text| text.starts_with("HOST clock_"))
            .collect()
    };
    let clock = crate::server::spawn_clock(std::sync::Arc::clone(&state));
    // The first tick is at once and moves nothing; the next waits for the interval.
    tokio::task::yield_now().await;
    tokio::time::advance(std::time::Duration::from_secs(1)).await;
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    let after_stall = lines(&state);
    assert_eq!(after_stall.len(), 2, "{after_stall:?}");
    assert!(
        after_stall[0].starts_with("HOST clock_stall gap_ms=1")
            && after_stall[0].ends_with(" cut_to_ms=250"),
        "{after_stall:?}"
    );
    assert!(
        after_stall[1].starts_with("HOST clock_idle ticks=2 ran_ms="),
        "{after_stall:?}"
    );
    {
        let request: crate::wire::EffectRequest = serde_json::from_value(
            json!({"op": "travel", "id": "m1", "x": 30.0, "y": 10.0, "speed": 100.0}),
        )
        .expect("an effect");
        let mut canvas = state.canvas();
        let effect = request.into_effect(&canvas).expect("converts");
        canvas.apply(effect, state.sink()).expect("applies");
    }
    state.wake_clock();
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    assert_eq!(
        lines(&state).last().map(String::as_str),
        Some("HOST clock_woken moving=1")
    );
    clock.abort();
    assert!(clock.await.is_err(), "the clock was aborted");
    assert_eq!(
        lines(&state).last().map(String::as_str),
        Some("HOST clock_stopped")
    );
}
