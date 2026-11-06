//! Travel from the command line to the canvas: the verbs, the host's clock and the frame a page
//! waits on.

use std::time::{Duration, Instant};

use axum::http::StatusCode;
use df_table::event::RejectionCause;

use crate::client::get;
use crate::command::{Command, CommandError, DEFAULT_TRAVEL_SPEED, parse, split_words, usage_text};
use crate::dev::{DevError, ReplyError, run_dev};
use crate::test_support::{Running, STARTER_FILES, start};
use crate::view::CanvasView;
use crate::wire::{EffectRequest, EventsReply, Frame};

fn words(line: &str) -> Vec<String> {
    split_words(line).expect("the line splits")
}

fn effects(line: &str) -> Vec<EffectRequest> {
    let Command::Effects(requests) = parse(&words(line)).expect("parses") else {
        panic!("expected effects");
    };
    requests
}

#[test]
fn travel_takes_an_element_a_point_and_an_optional_speed() {
    assert_eq!(
        effects("travel pip 40 20 speed=50"),
        vec![EffectRequest::Travel {
            id: "pip".into(),
            x: 40.0,
            y: 20.0,
            speed: 50.0
        }]
    );
    assert_eq!(
        effects("travel pip 40 20"),
        vec![EffectRequest::Travel {
            id: "pip".into(),
            x: 40.0,
            y: 20.0,
            speed: DEFAULT_TRAVEL_SPEED
        }]
    );
}

#[test]
fn travel_refuses_what_is_not_a_finite_number_an_unknown_key_or_too_few_words() {
    assert!(matches!(
        parse(&words("travel pip 40")),
        Err(CommandError::Usage(_))
    ));
    assert!(matches!(
        parse(&words("travel pip left 20")),
        Err(CommandError::BadNumber { .. })
    ));
    assert!(matches!(
        parse(&words("travel pip 1 2 speed=fast")),
        Err(CommandError::BadNumber { .. })
    ));
    assert!(matches!(
        parse(&words("travel pip 1 2 pace=3")),
        Err(CommandError::BadChoice { .. })
    ));
    assert_eq!(
        parse(&words("travel pip 1 2 speed=3 speed=4")),
        Err(CommandError::DuplicateKey("speed".into()))
    );
}

#[test]
fn stop_takes_several_elements_and_repl_takes_none_and_the_help_lists_them() {
    assert_eq!(effects("stop a b").len(), 2);
    assert!(matches!(parse(&words("stop")), Err(CommandError::Usage(_))));
    assert_eq!(parse(&words("repl")), Ok(Command::Repl));
    let help = usage_text();
    for text in ["travel <id>", "stop <id>", "repl"] {
        assert!(help.contains(text), "{text}");
    }
}

async fn run(running: &Running, line: &str) -> Result<String, DevError> {
    run_dev(running.port, &words(line)).await
}

async fn element_x(running: &Running, id: &str) -> f32 {
    let body = get(running.port, "/canvas.json")
        .await
        .expect("answers")
        .body;
    let view: CanvasView = serde_json::from_str(&body).expect("canvas");
    view.elements
        .iter()
        .find(|element| element.id == id)
        .expect("exists")
        .x
}

#[tokio::test]
async fn a_travelling_element_moves_a_step_each_tick_and_arrives_exactly() {
    let running = start(&STARTER_FILES).await;
    run(&running, "spawn marker m1 x=10 y=10")
        .await
        .expect("spawns");
    run(&running, "travel m1 20 10 speed=10")
        .await
        .expect("starts");
    running.state.tick(0.25);
    assert!((element_x(&running, "m1").await - 12.5).abs() < 1e-3);
    for _ in 0..4 {
        running.state.tick(0.25);
    }
    assert_eq!(element_x(&running, "m1").await, 20.0);
    let events = get(running.port, "/events.json").await.expect("answers");
    let events: EventsReply = serde_json::from_str(&events.body).expect("events");
    assert!(
        events
            .events
            .iter()
            .any(|line| line.text.starts_with("CANVAS arrive element=m1"))
    );
}

#[tokio::test]
async fn the_canvas_view_says_where_a_travelling_element_is_heading() {
    let running = start(&STARTER_FILES).await;
    run(&running, "spawn marker m1 x=10 y=10")
        .await
        .expect("spawns");
    run(&running, "travel m1 80 30 speed=15")
        .await
        .expect("starts");
    let body = get(running.port, "/canvas.json")
        .await
        .expect("answers")
        .body;
    let view: CanvasView = serde_json::from_str(&body).expect("canvas");
    let travelling = view.elements[0].travelling.expect("is travelling");
    assert_eq!(
        (travelling.x, travelling.y, travelling.speed),
        (80.0, 30.0, 15.0)
    );
    run(&running, "stop m1").await.expect("stops");
    let body = get(running.port, "/canvas.json")
        .await
        .expect("answers")
        .body;
    let view: CanvasView = serde_json::from_str(&body).expect("canvas");
    assert!(view.elements[0].travelling.is_none());
}

#[tokio::test]
async fn a_speed_that_is_not_allowed_is_refused_with_the_limits_and_recorded() {
    let running = start(&STARTER_FILES).await;
    run(&running, "spawn marker m1").await.expect("spawns");
    let error = run(&running, "travel m1 20 10 speed=0")
        .await
        .expect_err("refused");
    assert!(
        matches!(
            &error,
            DevError::Rejected {
                error: ReplyError::Canvas {
                    cause: Some(RejectionCause::InvalidSpeed),
                    ..
                },
                ..
            }
        ),
        "{error:?}"
    );
    let events = get(running.port, "/events.json").await.expect("answers");
    let events: EventsReply = serde_json::from_str(&events.body).expect("events");
    assert!(
        events
            .events
            .iter()
            .any(|line| line.text.contains("cause=invalid_speed"))
    );
}

#[tokio::test]
async fn the_hosts_own_clock_moves_a_travelling_element_without_any_help() {
    let mut running = start(&STARTER_FILES).await;
    running.start_clock();
    run(&running, "spawn marker m1 x=10 y=10")
        .await
        .expect("spawns");
    run(&running, "travel m1 30 10 speed=400")
        .await
        .expect("starts");
    let started = Instant::now();
    while element_x(&running, "m1").await < 30.0 {
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "the clock did not move it"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn frame(running: &Running, query: &str) -> Frame {
    let response = get(running.port, &format!("/playground/frame{query}"))
        .await
        .expect("answers");
    assert_eq!(response.status, StatusCode::OK);
    serde_json::from_str(&response.body).expect("a frame")
}

#[tokio::test]
async fn a_frame_without_after_answers_at_once_with_the_canvas_and_the_controls() {
    let running = start(&STARTER_FILES).await;
    run(&running, "spawn marker m1").await.expect("spawns");
    let first = frame(&running, "").await;
    assert_eq!(first.canvas.elements.len(), 1);
    assert!(first.frame >= 1, "the spawn was announced");
}

#[tokio::test]
async fn a_frame_asked_for_after_the_current_one_waits_for_the_next_change() {
    let running = start(&STARTER_FILES).await;
    let current = frame(&running, "").await.frame;
    let port = running.port;
    let waiting = tokio::spawn(async move {
        let started = Instant::now();
        let response = get(port, &format!("/playground/frame?after={current}"))
            .await
            .expect("answers");
        (
            started.elapsed(),
            serde_json::from_str::<Frame>(&response.body).expect("a frame"),
        )
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !waiting.is_finished(),
        "nothing changed, so it is still waiting"
    );
    run(&running, "spawn marker m1").await.expect("spawns");
    let (waited, answer) = waiting.await.expect("joins");
    assert!(
        waited >= Duration::from_millis(250) && waited < Duration::from_secs(5),
        "{waited:?}"
    );
    assert!(answer.frame > current);
    assert_eq!(
        answer.canvas.elements.len(),
        1,
        "the frame carries the change"
    );
}

#[tokio::test]
async fn a_frame_asked_for_after_an_older_count_answers_at_once() {
    let running = start(&STARTER_FILES).await;
    run(&running, "spawn marker m1").await.expect("spawns");
    let started = Instant::now();
    let answer = frame(&running, "?after=0").await;
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(answer.frame > 0);
    let ahead = frame(&running, "?after=999999").await;
    assert_eq!(
        ahead.frame, answer.frame,
        "a host that restarted answers with its own count at once"
    );
}

#[tokio::test]
async fn every_kind_of_change_wakes_a_waiting_page() {
    let running = start(&STARTER_FILES).await;
    for line in [
        "spawn character c1 name=Pip",
        "move c1 5 5",
        "set c1 hit_points=3",
        "playground set grid=on",
    ] {
        let before = frame(&running, "").await.frame;
        run(&running, line).await.expect("applies");
        assert!(frame(&running, "").await.frame > before, "{line}");
    }
}

#[tokio::test]
async fn a_tick_that_moves_something_wakes_a_waiting_page_and_an_idle_one_does_not() {
    let running = start(&STARTER_FILES).await;
    run(&running, "spawn marker m1 x=10 y=10")
        .await
        .expect("spawns");
    let idle = frame(&running, "").await.frame;
    running.state.tick(0.1);
    assert_eq!(
        frame(&running, "").await.frame,
        idle,
        "idle ticks announce nothing"
    );
    run(&running, "travel m1 50 10 speed=10")
        .await
        .expect("starts");
    let started = frame(&running, "").await.frame;
    running.state.tick(0.1);
    assert!(frame(&running, "").await.frame > started);
}
