//! The frame is built once for each change and shared; it carries no kinds; the copy of the canvas
//! finds each element's problem as it goes; and the host's clock sleeps while nothing moves.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use df_table::canvas::{Canvas, CanvasEffect};
use df_table::element::{ElementId, FieldValue, KindName, Position};
use df_table::kind::KindRegistry;
use df_table::testkit::RecordingSink;

use crate::client::get;
use crate::command::split_words;
use crate::dev::run_dev;
use crate::test_support::{Running, STARTER_FILES, start};
use crate::view::{CanvasView, FrameCanvas, ViewError, value_json};
use crate::wire::Frame;

async fn run(running: &Running, line: &str) {
    run_dev(running.port, &split_words(line).expect("the line splits"))
        .await
        .expect("the command runs");
}

#[tokio::test]
async fn one_change_count_is_built_once_and_every_reader_gets_the_same_bytes() {
    let running = start(&STARTER_FILES).await;
    run(&running, "spawn marker m1").await;
    let first = running.state.frame().await.expect("builds");
    let second = running.state.frame().await.expect("builds");
    assert!(
        Arc::ptr_eq(&first.canvas, &second.canvas),
        "the second reader shares the first's canvas bytes"
    );
    run(&running, "spawn marker m2").await;
    let third = running.state.frame().await.expect("builds");
    assert!(
        !Arc::ptr_eq(&first.canvas, &third.canvas),
        "a change builds a new canvas"
    );
}

#[tokio::test]
async fn five_pages_asking_for_the_same_frame_are_sent_identical_bytes() {
    let running = start(&STARTER_FILES).await;
    run(&running, "spawn marker m1").await;
    let mut bodies = Vec::new();
    for _ in 0..5 {
        bodies.push(
            get(running.port, "/playground/frame")
                .await
                .expect("answers")
                .body,
        );
    }
    assert!(bodies.windows(2).all(|pair| pair[0] == pair[1]));
}

#[tokio::test]
async fn a_frame_carries_the_revision_and_elements_but_not_the_kinds() {
    let running = start(&STARTER_FILES).await;
    run(&running, "spawn marker m1").await;
    let body = get(running.port, "/playground/frame")
        .await
        .expect("answers")
        .body;
    let json: serde_json::Value = serde_json::from_str(&body).expect("json");
    let canvas = json
        .get("canvas")
        .and_then(|canvas| canvas.as_object())
        .expect("a canvas");
    let mut keys: Vec<&str> = canvas.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(keys, ["elements", "revision"]);
    assert!(!body.contains("\"kinds\""));
    let frame: Frame = serde_json::from_str(&body).expect("a frame");
    assert_eq!(frame.canvas.elements.len(), 1);
    let full = get(running.port, "/canvas.json")
        .await
        .expect("answers")
        .body;
    assert!(
        full.contains("\"kinds\""),
        "canvas.json still lists the kinds"
    );
}

#[test]
fn every_element_of_a_copy_names_its_own_problem() {
    let mut canvas = Canvas::new(df_table::testkit::registry_from(&[
        df_table::testkit::CHARACTER_KIND,
        df_table::testkit::MARKER_KIND,
    ]));
    let sink = RecordingSink::default();
    for index in 0..300 {
        canvas
            .apply(
                CanvasEffect::Spawn {
                    id: ElementId::new(&format!("m{index}")).expect("valid id"),
                    kind: KindName::new("marker").expect("valid kind"),
                    position: Position::new(10.0, 10.0),
                    fields: BTreeMap::new(),
                },
                &sink,
            )
            .expect("spawns");
    }
    let healthy = FrameCanvas::from_canvas(&canvas).expect("copies");
    assert!(
        healthy
            .elements
            .iter()
            .all(|element| element.problem.is_none())
    );
    canvas.replace_registry(KindRegistry::default(), &sink);
    let broken = CanvasView::from_canvas(&canvas).expect("copies");
    assert_eq!(broken.elements.len(), 300);
    assert!(
        broken.elements.iter().all(|element| element
            .problem
            .as_deref()
            .is_some_and(|text| text.contains("no kind"))),
        "each element carries its problem"
    );
    assert!(broken.kinds.is_empty());
}

#[test]
fn a_number_that_is_not_finite_is_not_written_as_null() {
    assert_eq!(value_json(&FieldValue::Number(f64::NAN)), None);
    assert_eq!(value_json(&FieldValue::Number(f64::INFINITY)), None);
    assert_eq!(
        value_json(&FieldValue::Number(2.5)),
        Some(serde_json::json!(2.5))
    );
    let error = ViewError::NotWritable {
        element: "pip".to_owned(),
        field: "hit_points".to_owned(),
    };
    assert!(error.to_string().contains("pip") && error.to_string().contains("hit_points"));
}

#[tokio::test]
async fn an_idle_clock_never_takes_the_canvas_lock() {
    let mut running = start(&STARTER_FILES).await;
    run(&running, "spawn marker m1").await;
    running.start_clock();
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(
        running.state.tick_count(),
        0,
        "about twelve ticks would have run if the clock still woke 30 times a second"
    );
}

#[tokio::test]
async fn the_clock_wakes_for_a_travel_and_goes_back_to_sleep_when_it_arrives() {
    let mut running = start(&STARTER_FILES).await;
    run(&running, "spawn marker m1 x=10 y=10").await;
    running.start_clock();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(running.state.tick_count(), 0);
    run(&running, "travel m1 14 10 speed=40").await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let ticks = running.state.tick_count();
    assert!(ticks > 0, "the travel woke the clock");
    assert!(!running.state.is_moving(), "and the element arrived");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        running.state.tick_count(),
        ticks,
        "an arrived canvas lets the clock sleep again"
    );
    run(&running, "travel m1 10 10 speed=40").await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(
        running.state.tick_count() > ticks,
        "a second travel wakes it again"
    );
}
