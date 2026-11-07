//! The second review of the host: races in the controls and the fixture write, the order and the
//! reasons of the audit lines, the echo writer's end of life, bounds by design, the glide of a
//! turn and the ranges the crates share.

use std::sync::Arc;
use std::time::Duration;

use axum::http::StatusCode;
use df_playground::controls::ControlsChange;
use df_playground::fixture::StageElement;
use serde_json::json;

use crate::client::{get, post_json};
use crate::command::split_words;
use crate::dev::{DevError, ReplyError, run_dev};
use crate::event_log::EventLog;
use crate::limits::{
    ECHO_LINE_OVERHEAD_BYTES, FIGURE_GLIDE_MS, MAXIMUM_BATCH_CHANGES, TURN_SETTLE,
};
use crate::playground::{ControlsChangeError, PlaygroundFiles, PlaygroundState};
use crate::sink::LineEcho;
use crate::test_support::{Running, STARTER_FILES, start};
use crate::wire::EventsReply;
use df_playground::limits::MAXIMUM_REPORT_BYTES;

const SCENE: &str = r#"{
    "name": "A scene",
    "tv": {"place": "Here", "status": {"kind": "ok"}, "elements": []},
    "phone": {"hero": "Pip", "status": {"kind": "ok"}}
}"#;

async fn run(running: &Running, line: &str) -> Result<String, DevError> {
    run_dev(running.port, &split_words(line).expect("the line splits")).await
}

async fn log_lines(running: &Running) -> Vec<String> {
    let response = get(running.port, "/events.json").await.expect("answers");
    let reply: EventsReply = serde_json::from_str(&response.body).expect("events");
    reply.events.into_iter().map(|line| line.text).collect()
}

/// Everything the host answers to `request`, as bytes.
async fn raw_bytes(port: u16, request: &str) -> Vec<u8> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connects");
    stream.write_all(request.as_bytes()).await.expect("sends");
    let mut answer = Vec::new();
    stream.read_to_end(&mut answer).await.expect("reads");
    answer
}

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|name| (*name).to_owned()).collect()
}

fn state() -> PlaygroundState {
    PlaygroundState::new(
        PlaygroundFiles::new("assets".into(), "pkg".into()),
        "test-boot".to_owned(),
    )
}

// 1671: the fixture check is made against the files as they were read, under the lock.

#[test]
fn a_choice_of_fixture_read_before_a_removal_is_refused_as_changed() {
    let state = state();
    let names = names(&["a.json", "b.json"]);
    state.reconcile(&names, |_| {});
    let generation = state.generation();
    // `fixture remove b` completes between reading the names and applying the choice.
    state.fixture_files_changed();
    let error = state
        .change_all(
            vec![ControlsChange::Fixture("b.json".to_owned())],
            &names,
            generation,
            |_| panic!("nothing is applied"),
        )
        .expect_err("refused");
    assert_eq!(error, ControlsChangeError::FixtureChanged);
    assert_eq!(error.status().as_u16(), 409);
    assert_eq!(error.key(), "fixture_changed");
    assert_eq!(
        state.snapshot().controls.fixture.as_deref(),
        Some("a.json"),
        "b.json was not shown"
    );
}

#[test]
fn a_change_that_does_not_choose_a_fixture_is_not_held_up_by_a_changed_generation() {
    let state = state();
    let names = names(&["a.json"]);
    state.reconcile(&names, |_| {});
    let generation = state.generation();
    state.fixture_files_changed();
    assert!(
        state
            .change_all(
                vec![ControlsChange::ShowGrid(true)],
                &names,
                generation,
                |_| {}
            )
            .is_ok()
    );
}

#[tokio::test]
async fn sixteen_writes_of_one_fixture_at_once_each_succeed_and_leave_one_whole_file() {
    let running = start(&STARTER_FILES).await;
    let texts: Vec<String> = (0..16)
        .map(|index| SCENE.replace("A scene", &format!("Scene number {index}")))
        .collect();
    let writes = texts.iter().map(|text| {
        let body = json!({"name": "tavern", "text": text}).to_string();
        let port = running.port;
        async move { post_json(port, "/dev/playground/fixture", &body).await }
    });
    for answer in futures_util::future::join_all(writes).await {
        assert_eq!(answer.expect("answers").status, StatusCode::OK);
    }
    let landed = std::fs::read_to_string(running.playground_path("assets/fixtures/tavern.json"))
        .expect("the file is there");
    assert!(
        texts.contains(&landed),
        "the file is one of the texts, whole"
    );
    assert!(df_playground::fixture::Fixture::parse(&landed).is_ok());
    let leftovers: Vec<_> = std::fs::read_dir(running.playground_path("assets/fixtures"))
        .expect("lists")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".tmp"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

// 1672: one line per counted change, in revision order, with reasons for refusals.

#[test]
fn the_audit_callback_runs_with_the_lock_so_lines_come_in_revision_order() {
    let state = Arc::new(state());
    let log = Arc::new(EventLog::default());
    let threads: Vec<_> = (0..8_u64)
        .map(|thread| {
            let (state, log) = (Arc::clone(&state), Arc::clone(&log));
            std::thread::spawn(move || {
                for step in 0..60_u64 {
                    // Every value is new, so every change counts.
                    let ms = 1 + thread * 100 + step;
                    state
                        .change_all(vec![ControlsChange::TransitionMs(ms)], &[], 0, |reply| {
                            log.push(format!("{}", reply.revision))
                        })
                        .expect("applies");
                }
            })
        })
        .collect();
    for thread in threads {
        thread.join().expect("finishes");
    }
    let revisions: Vec<u64> = log
        .newest(10_000)
        .iter()
        .map(|line| line.text.parse().expect("a revision"))
        .collect();
    assert_eq!(revisions.len(), 480);
    assert!(
        revisions.windows(2).all(|pair| pair[0] + 1 == pair[1]),
        "the log order is the revision order, with none missing"
    );
}

#[tokio::test]
async fn a_change_that_alters_nothing_and_an_empty_batch_write_no_set_line() {
    let running = start(&STARTER_FILES).await;
    run(&running, "playground set grid=true")
        .await
        .expect("sets");
    run(&running, "playground set grid=true")
        .await
        .expect("sets again");
    let response = post_json(
        running.port,
        "/dev/playground/controls/batch",
        r#"{"changes":[]}"#,
    )
    .await
    .expect("answers");
    assert_eq!(response.status, StatusCode::BAD_REQUEST);
    assert!(
        response.body.contains("at least one change"),
        "{}",
        response.body
    );
    let lines = log_lines(&running).await;
    assert_eq!(
        lines
            .iter()
            .filter(|line| line.starts_with("PLAYGROUND set "))
            .count(),
        1,
        "only the change that counted: {lines:?}"
    );
    // The line ends with who asked, which the attribution tests check.
    assert!(
        lines.iter().any(|line| line
            .starts_with("PLAYGROUND reject cause=invalid_value reason=empty_batch source=")),
        "{lines:?}"
    );
}

#[tokio::test]
async fn a_refusal_names_its_reason_as_a_token() {
    let running = start(&STARTER_FILES).await;
    running.write_asset("fixtures/a.json", SCENE);
    run(&running, "playground set status=sleepy")
        .await
        .expect_err("refused");
    run(&running, "playground go castle")
        .await
        .expect_err("refused");
    let lines = log_lines(&running).await;
    for reason in ["unknown_status", "no_such_fixture"] {
        // The line ends with who asked, which the attribution tests check.
        let wanted = format!("PLAYGROUND reject cause=invalid_value reason={reason} source=");
        assert!(
            lines.iter().any(|line| line.starts_with(&wanted)),
            "{reason}: {lines:?}"
        );
    }
}

#[tokio::test]
async fn a_batch_of_too_many_changes_is_refused_and_one_at_the_most_makes_a_bounded_line() {
    let running = start(&STARTER_FILES).await;
    let changes = |count: usize| {
        let list: Vec<_> = (1..=count)
            .map(|ms| json!({"change": "transition_ms", "value": ms}))
            .collect();
        json!({ "changes": list }).to_string()
    };
    let too_many = post_json(
        running.port,
        "/dev/playground/controls/batch",
        &changes(MAXIMUM_BATCH_CHANGES + 1),
    )
    .await
    .expect("answers");
    assert_eq!(too_many.status, StatusCode::BAD_REQUEST);
    assert!(too_many.body.contains("at most"), "{}", too_many.body);
    let allowed = post_json(
        running.port,
        "/dev/playground/controls/batch",
        &changes(MAXIMUM_BATCH_CHANGES),
    )
    .await
    .expect("answers");
    assert_eq!(allowed.status, StatusCode::OK);
    let lines = log_lines(&running).await;
    let longest = lines.iter().map(String::len).max().expect("a line");
    // Each change writes one short pair (` transition_ms=100000`), well under 64 bytes.
    let bound = 64 * MAXIMUM_BATCH_CHANGES;
    assert!(longest < bound, "the longest line is {longest} bytes");
    assert!(
        lines
            .iter()
            .any(|line| line.contains("reason=too_many_changes"))
    );
}

// 1673: the echo writer's end of life.

#[test]
fn finish_waits_for_the_writer_to_print_what_is_queued() {
    let log = Arc::new(EventLog::default());
    let printed = Arc::new(std::sync::Mutex::new(Vec::new()));
    let writer = Arc::clone(&printed);
    let echo = LineEcho::starting_after(&log, move |line| {
        std::thread::sleep(Duration::from_millis(15));
        writer.lock().expect("not poisoned").push(line.to_owned());
    });
    for number in 0..20 {
        log.push(format!("line {number}"));
    }
    echo.echo_new(&log);
    assert!(echo.finish(Duration::from_secs(10)), "the writer ended");
    assert_eq!(
        printed.lock().expect("not poisoned").len(),
        20,
        "all of them"
    );
}

#[test]
fn finish_gives_up_on_a_terminal_that_never_takes_a_line() {
    let log = Arc::new(EventLog::default());
    let (release, hold) = std::sync::mpsc::channel::<()>();
    let echo = LineEcho::starting_after(&log, move |_| {
        let _ = hold.recv_timeout(Duration::from_secs(10));
    });
    log.push("stuck".to_owned());
    echo.echo_new(&log);
    let started = std::time::Instant::now();
    assert!(!echo.finish(Duration::from_millis(200)));
    assert!(started.elapsed() < Duration::from_secs(5));
    release.send(()).expect("lets go");
}

#[test]
fn a_writer_that_died_is_reported_once_in_the_log_and_its_lines_are_counted() {
    let log = Arc::new(EventLog::default());
    let echo = LineEcho::starting_after(&log, |_| panic!("the terminal is gone"));
    log.push("first".to_owned());
    echo.echo_new(&log);
    // Give the writer time to die, then keep logging.
    std::thread::sleep(Duration::from_millis(200));
    for number in 0..3 {
        log.push(format!("later {number}"));
        echo.echo_new(&log);
    }
    assert!(echo.writer_gone());
    let reports = log
        .newest(100)
        .into_iter()
        .filter(|line| line.text.starts_with("HOST echo_writer_gone"))
        .count();
    assert_eq!(reports, 1, "said once");
    assert!(echo.dropped_lines() >= 3);
}

#[test]
fn lines_the_log_forgot_before_the_echo_looked_are_counted_as_dropped() {
    let log = Arc::new(EventLog::with_bounds(5, usize::MAX));
    let (release, hold) = std::sync::mpsc::channel::<()>();
    let echo = LineEcho::starting_after(&log, move |_| {
        let _ = hold.recv_timeout(Duration::from_secs(10));
    });
    for number in 0..20 {
        log.push(format!("line {number}"));
    }
    echo.echo_new(&log);
    assert_eq!(echo.dropped_lines(), 15, "20 logged, 5 kept, 15 forgotten");
    release.send(()).expect("lets go");
}

#[test]
fn the_notice_of_dropped_lines_is_printed_even_when_nothing_else_follows() {
    let log = Arc::new(EventLog::default());
    let (tell, heard) = std::sync::mpsc::channel::<String>();
    let line_cost = "x".len() + ECHO_LINE_OVERHEAD_BYTES;
    // Room for exactly one line: the second is dropped, and nothing is logged after it.
    let echo = LineEcho::bounded(&log, line_cost + 1, move |line| {
        let _ = tell.send(line.to_owned());
    });
    log.push("x".to_owned());
    log.push("y".to_owned());
    echo.echo_new(&log);
    let mut lines = Vec::new();
    while let Ok(line) = heard.recv_timeout(Duration::from_secs(3)) {
        lines.push(line);
        if lines.iter().any(|line| line.contains("not echoed")) {
            break;
        }
    }
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with("1 audit lines were not echoed")),
        "{lines:?}"
    );
}

// 1674: bounds by design.

#[test]
fn the_log_is_bounded_in_bytes_as_well_as_in_lines() {
    let log = EventLog::with_bounds(1_000, 100);
    for number in 0..50 {
        log.push(format!("{number:02} {}", "x".repeat(27)));
    }
    let kept = log.newest(1_000);
    assert!(
        kept.len() <= 3,
        "{} lines of 30 bytes in a 100-byte log",
        kept.len()
    );
    assert_eq!(kept.last().expect("a line").sequence, 50);
}

#[tokio::test]
async fn a_report_larger_than_the_most_a_page_may_send_is_refused() {
    let running = start(&STARTER_FILES).await;
    let body = json!({
        "client": "page-1",
        "report": {"errors": ["x".repeat(MAXIMUM_REPORT_BYTES)]}
    })
    .to_string();
    let response = post_json(running.port, "/dev/playground/report", &body)
        .await
        .expect("answers");
    assert_eq!(response.status, StatusCode::PAYLOAD_TOO_LARGE);
    assert!(running.state.playground().clients().is_empty());
}

#[tokio::test]
async fn the_package_is_streamed_whole_and_a_page_file_over_its_bound_is_refused() {
    let running = start(&STARTER_FILES).await;
    let bytes: Vec<u8> = (0..3_000_000_u32)
        .map(|index| (index % 251) as u8)
        .collect();
    running.write_package_file("app_bg.wasm", &bytes);
    let answer = raw_bytes(
        running.port,
        "GET /playground/pkg/app_bg.wasm HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
    )
    .await;
    let split = answer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("a head");
    let head = String::from_utf8_lossy(&answer[..split]).to_ascii_lowercase();
    assert!(head.starts_with("http/1.1 200"), "{head}");
    assert!(head.contains("content-length: 3000000"), "{head}");
    assert_eq!(
        &answer[split + 4..],
        bytes.as_slice(),
        "every byte, in order"
    );
    // A theme larger than a page file may be is refused instead of read whole.
    let theme = running.playground_path("assets/theme.css");
    std::fs::create_dir_all(theme.parent().expect("a parent")).expect("makes it");
    let big = std::fs::File::create(&theme).expect("creates");
    big.set_len(crate::limits::MAXIMUM_ASSET_FILE_BYTES + 1)
        .expect("a sparse file");
    drop(big);
    let refused = get(running.port, "/playground/theme.css")
        .await
        .expect("answers");
    assert_eq!(refused.status, StatusCode::PAYLOAD_TOO_LARGE);
}

// 1678: the glide of a turn.

#[tokio::test]
async fn a_pivot_given_during_the_glide_of_the_turn_back_is_refused_and_after_it_is_accepted() {
    let running = start(&STARTER_FILES).await;
    run(&running, "spawn marker m1").await.expect("spawns");
    run(&running, "rotate m1 90").await.expect("turns");
    tokio::time::sleep(TURN_SETTLE + Duration::from_millis(60)).await;
    run(&running, "rotate m1 0").await.expect("turns back");
    let error = run(&running, "rotate m1 0 pivot=0,0")
        .await
        .expect_err("still gliding");
    assert!(
        matches!(
            &error,
            DevError::Rejected {
                error: ReplyError::Canvas {
                    cause: Some(df_table::event::RejectionCause::InvalidTransform),
                    ..
                },
                // The cause does not tell a glide from a turned element yet; the sentence does.
                message,
            } if message.contains("try again in")
        ),
        "{error:?}"
    );
    assert!(
        log_lines(&running).await.contains(
            &"CANVAS reject effect=transform cause=invalid_transform element=m1".to_owned()
        )
    );
    run(&running, "rotate m1 0 pivot=50,50")
        .await
        .expect("the same pivot may be given");
    tokio::time::sleep(TURN_SETTLE + Duration::from_millis(60)).await;
    run(&running, "rotate m1 0 pivot=0,0")
        .await
        .expect("after the glide it may");
}

#[tokio::test]
async fn an_element_never_turned_may_take_a_pivot_at_once() {
    let running = start(&STARTER_FILES).await;
    run(&running, "spawn marker m1").await.expect("spawns");
    run(&running, "rotate m1 0 pivot=0,0")
        .await
        .expect("nothing is gliding");
}

#[test]
fn the_settle_time_covers_the_glide_the_theme_declares() {
    let theme = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../assets/playground/theme.css"
    ))
    .expect("the theme is there");
    assert!(
        theme.contains(&format!("transform {FIGURE_GLIDE_MS}ms linear")),
        "the figure no longer glides a turn over {FIGURE_GLIDE_MS} ms"
    );
    assert!(TURN_SETTLE > Duration::from_millis(FIGURE_GLIDE_MS));
}

// 1649, 1680: a turn then a travel at the wire, and the ranges the crates share.

async fn frame_elements(running: &Running) -> Vec<StageElement> {
    let body = get(running.port, "/playground/frame")
        .await
        .expect("answers")
        .body;
    let json: serde_json::Value = serde_json::from_str(&body).expect("json");
    serde_json::from_value(json["canvas"]["elements"].clone()).expect("the page reads it")
}

#[tokio::test]
async fn a_turned_element_that_travels_keeps_its_turn_and_pivot_and_arrives() {
    let mut running = start(&STARTER_FILES).await;
    running.start_clock();
    run(&running, "spawn marker m1 x=10 y=10")
        .await
        .expect("spawns");
    run(&running, "rotate m1 0 pivot=0,50")
        .await
        .expect("a hinge");
    run(&running, "rotate m1 90").await.expect("turns");
    let body = json!({"op": "travel", "id": "m1", "x": 14.0, "y": 10.0, "speed": 40.0});
    let answer = post_json(running.port, "/dev/effect", &body.to_string())
        .await
        .expect("answers");
    assert_eq!(answer.status, StatusCode::OK);
    for _ in 0..100 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        if !running.state.is_moving() {
            break;
        }
    }
    let element = &frame_elements(&running).await[0];
    assert_eq!((element.x, element.y), (14.0, 10.0), "it arrived");
    assert_eq!(element.rotation_degrees, 90, "travel did not turn it back");
    assert_eq!(
        (element.pivot.x_percent, element.pivot.y_percent),
        (0, 50),
        "travel did not move its pivot"
    );
}

#[test]
fn the_two_crates_agree_on_the_turn_and_share_ranges() {
    assert_eq!(
        df_table::element::MAXIMUM_ROTATION_DEGREES,
        df_playground::limits::MAXIMUM_ROTATION_DEGREES
    );
    assert_eq!(
        df_table::element::MAXIMUM_PERCENT,
        df_playground::limits::MAXIMUM_PIVOT_PERCENT
    );
    assert_eq!(
        df_table::element::MAXIMUM_PERCENT,
        df_playground::limits::MAXIMUM_SLIDE_PERCENT
    );
}

#[tokio::test]
async fn the_pivot_refusal_on_a_turned_element_keeps_its_own_message() {
    let running = start(&STARTER_FILES).await;
    run(&running, "spawn marker m1").await.expect("spawns");
    run(&running, "rotate m1 90").await.expect("turns");
    let error = run(&running, "rotate m1 90 pivot=0,0")
        .await
        .expect_err("refused");
    assert!(error.to_string().contains("rotate m1 0"), "{error}");
}

// 1627: the bodiless answer to a wait that ran out, for a page that asks for it.

#[tokio::test]
async fn a_wait_that_runs_out_answers_204_only_for_a_page_that_asked_for_it() {
    let running = start(&STARTER_FILES).await;
    let count = running.state.change_count();
    let patience = Duration::from_millis(60);
    let quiet = crate::server::FrameQuery::for_test(Some(count), Some(204));
    let answer = crate::server::frame_answer_for_test(&running.state, &quiet, patience).await;
    assert_eq!(
        answer.status().as_u16(),
        204,
        "nothing changed, and the page asked"
    );
    let plain = crate::server::FrameQuery::for_test(Some(count), None);
    let answer = crate::server::frame_answer_for_test(&running.state, &plain, patience).await;
    assert_eq!(
        answer.status().as_u16(),
        200,
        "a page that did not ask still gets the frame"
    );
    run(&running, "spawn marker m1").await.expect("spawns");
    let answer = crate::server::frame_answer_for_test(&running.state, &quiet, patience).await;
    assert_eq!(
        answer.status().as_u16(),
        200,
        "after a change the frame is sent"
    );
}
