//! The host's JSON and the page's types must agree: the page reads the frame's elements straight
//! into `StageElement`, and both sides default the newer fields, so a rename on one side would make
//! every figure draw unturned with no error. These tests tie the two, and pin the pivot rule that
//! keeps a turned figure from jumping.

use df_playground::fixture::{ElementShape, StageElement};
use df_table::kind::Shape;

use crate::client::get;
use crate::command::split_words;
use crate::dev::{DevError, run_dev};
use crate::test_support::{Running, STARTER_FILES, start};
use crate::view::{ElementView, PivotView};

fn page_shape(shape: Shape) -> ElementShape {
    match shape {
        Shape::Circle => ElementShape::Circle,
        Shape::Rect => ElementShape::Rect,
    }
}

fn host_shape(shape: ElementShape) -> Shape {
    match shape {
        ElementShape::Circle => Shape::Circle,
        ElementShape::Rect => Shape::Rect,
    }
}

#[test]
fn the_two_shape_enums_name_the_same_shapes_in_json() {
    for shape in [Shape::Circle, Shape::Rect] {
        let host = serde_json::to_value(shape).expect("writes");
        let page = serde_json::to_value(page_shape(shape)).expect("writes");
        assert_eq!(host, page);
        assert_eq!(host_shape(page_shape(shape)), shape);
    }
    for shape in [ElementShape::Circle, ElementShape::Rect] {
        assert_eq!(page_shape(host_shape(shape)), shape);
    }
}

fn view(rotation_degrees: u16, x_percent: u8, y_percent: u8) -> ElementView {
    ElementView {
        id: "door".to_owned(),
        kind: "marker".to_owned(),
        x: 40.0,
        y: 20.0,
        label: "door".to_owned(),
        color: Some("#ff8800".to_owned()),
        shape: Shape::Rect,
        width_units: 8.0,
        height_units: 2.0,
        fields: serde_json::Map::new(),
        problem: None,
        travelling: None,
        rotation_degrees,
        pivot: PivotView {
            x_percent,
            y_percent,
        },
    }
}

#[test]
fn an_element_view_with_a_turn_and_a_pivot_reads_as_the_pages_stage_element() {
    let json = serde_json::to_value(view(270, 0, 50)).expect("writes");
    let stage: StageElement = serde_json::from_value(json).expect("the page reads it");
    assert_eq!(stage.rotation_degrees, 270);
    assert_eq!(
        (stage.pivot.x_percent, stage.pivot.y_percent),
        (0, 50),
        "a renamed pivot field would read as the centre"
    );
    assert_eq!((stage.x, stage.y), (40.0, 20.0));
    assert_eq!(stage.shape, ElementShape::Rect);
    assert_eq!(stage.color.as_deref(), Some("#ff8800"));
}

async fn run(running: &Running, line: &str) -> Result<String, DevError> {
    run_dev(running.port, &split_words(line).expect("the line splits")).await
}

async fn frame_elements(running: &Running) -> Vec<StageElement> {
    let body = get(running.port, "/playground/frame")
        .await
        .expect("answers")
        .body;
    let json: serde_json::Value = serde_json::from_str(&body).expect("json");
    serde_json::from_value(json["canvas"]["elements"].clone()).expect("the page reads the frame")
}

#[tokio::test]
async fn a_live_frame_reads_as_stage_elements_with_the_rotation_and_pivot_the_host_holds() {
    let running = start(&STARTER_FILES).await;
    run(&running, "spawn marker m1 x=30 y=12")
        .await
        .expect("spawns");
    run(&running, "rotate m1 0 pivot=100,0")
        .await
        .expect("a pivot at rotation 0");
    run(&running, "rotate m1 90").await.expect("turns");
    let elements = frame_elements(&running).await;
    assert_eq!(elements.len(), 1);
    assert_eq!(elements[0].rotation_degrees, 90);
    assert_eq!(
        (elements[0].pivot.x_percent, elements[0].pivot.y_percent),
        (100, 0)
    );
}

/// Where the page draws the middle of a figure: its position, moved by what turning about the
/// pivot does to the middle, `(I - R)(pivot - middle)` (the page centres the figure on its
/// position, then turns it clockwise about `transform-origin`).
fn drawn_middle(element: &StageElement) -> (f32, f32) {
    let width = element.width_units;
    let height = match element.shape {
        ElementShape::Circle => element.width_units,
        ElementShape::Rect => element.height_units,
    };
    let arm = (
        (f32::from(element.pivot.x_percent) - 50.0) / 100.0 * width,
        (f32::from(element.pivot.y_percent) - 50.0) / 100.0 * height,
    );
    let turn = f32::from(element.rotation_degrees).to_radians();
    let (sine, cosine) = turn.sin_cos();
    let turned = (cosine * arm.0 - sine * arm.1, sine * arm.0 + cosine * arm.1);
    (element.x + arm.0 - turned.0, element.y + arm.1 - turned.1)
}

fn apart(left: (f32, f32), right: (f32, f32)) -> f32 {
    ((left.0 - right.0).powi(2) + (left.1 - right.1).powi(2)).sqrt()
}

#[tokio::test]
async fn a_new_pivot_on_a_turned_figure_would_jump_so_it_is_refused_and_nothing_moves() {
    let running = start(&STARTER_FILES).await;
    run(&running, "spawn marker m1 x=30 y=12")
        .await
        .expect("spawns");
    run(&running, "rotate m1 90")
        .await
        .expect("turns about the centre");
    let before = frame_elements(&running).await;
    let drawn_before = drawn_middle(&before[0]);

    // What the page would draw with the new pivot and the old position: the jump the rule prevents.
    let mut hinged = before[0].clone();
    hinged.pivot.x_percent = 0;
    assert!(
        apart(drawn_before, drawn_middle(&hinged)) > 0.5,
        "the figure would jump by {} units",
        apart(drawn_before, drawn_middle(&hinged))
    );

    let error = run(&running, "rotate m1 90 pivot=0,50")
        .await
        .expect_err("refused");
    assert!(
        matches!(&error, DevError::Rejected(text) if text.contains("rotate m1 0")),
        "{error}"
    );
    let after = frame_elements(&running).await;
    assert_eq!(after, before, "the frame is exactly as it was");
    assert!(apart(drawn_before, drawn_middle(&after[0])) < 1e-4);
}

#[tokio::test]
async fn turning_back_to_zero_first_lets_the_pivot_change_without_a_jump() {
    let running = start(&STARTER_FILES).await;
    run(&running, "spawn marker m1 x=30 y=12")
        .await
        .expect("spawns");
    run(&running, "rotate m1 90").await.expect("turns");
    run(&running, "rotate m1 0").await.expect("turns back");
    let level = frame_elements(&running).await;
    run(&running, "rotate m1 0 pivot=0,50")
        .await
        .expect("a pivot at rotation 0");
    let hinged = frame_elements(&running).await;
    assert!(
        apart(drawn_middle(&level[0]), drawn_middle(&hinged[0])) < 1e-4,
        "at rotation 0 the pivot changes nothing that is drawn"
    );
    run(&running, "rotate m1 90")
        .await
        .expect("turns about the new hinge");
    let turned = frame_elements(&running).await;
    assert_eq!(
        (turned[0].x, turned[0].y),
        (level[0].x, level[0].y),
        "turning moves no position"
    );
    assert_eq!(turned[0].pivot.x_percent, 0);
}
