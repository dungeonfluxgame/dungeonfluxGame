//! Rotating an element from the command line, the wire and a script to the canvas, the log and
//! the frame a page waits on (O.13 T0).

use axum::http::StatusCode;
use df_table::event::RejectionCause;

use crate::client::{get, post_json};
use crate::command::{Command, CommandError, parse, split_words, usage_text};
use crate::dev::{DevError, ReplyError, run_dev};
use crate::repl::{LineResult, run_line};
use crate::test_support::{Running, STARTER_FILES, start};
use crate::view::{CanvasView, ElementView, PivotView};
use crate::wire::{EffectRequest, EventsReply, Frame, PivotRequest, Reply};

fn words(line: &str) -> Vec<String> {
    split_words(line).expect("the line splits")
}

fn effects(line: &str) -> Vec<EffectRequest> {
    let Command::Effects(requests) = parse(&words(line)).expect("parses") else {
        panic!("expected effects");
    };
    requests
}

fn transform(id: &str, rotation_degrees: i64, pivot: Option<PivotRequest>) -> EffectRequest {
    EffectRequest::Transform {
        id: id.into(),
        rotation_degrees: serde_json::Number::from(rotation_degrees),
        pivot,
    }
}

fn hinge() -> PivotRequest {
    PivotRequest {
        x_percent: 0.into(),
        y_percent: 50.into(),
    }
}

#[test]
fn rotate_takes_an_element_degrees_and_an_optional_pivot() {
    assert_eq!(effects("rotate pip 90"), vec![transform("pip", 90, None)]);
    assert_eq!(
        effects("rotate door 359 pivot=0,50"),
        vec![transform("door", 359, Some(hinge()))]
    );
    assert_eq!(effects("rotate pip 0"), vec![transform("pip", 0, None)]);
}

fn range_refusal(line: &str, key: &str, range: &str) {
    match parse(&words(line)) {
        Err(error @ CommandError::BadChoice { .. }) => {
            let CommandError::BadChoice { key: refused, .. } = &error else {
                unreachable!("matched above");
            };
            assert_eq!(refused, key, "{line}");
            assert!(error.to_string().contains(range), "{line}: {error}");
        }
        other => panic!("{line}: expected a refusal naming the range, got {other:?}"),
    }
}

#[test]
fn rotate_refuses_degrees_outside_zero_to_359_and_names_the_range() {
    for line in [
        "rotate pip 360",
        "rotate pip -1",
        "rotate pip 90.5",
        "rotate pip ninety",
        "rotate pip 99999999999999999999",
    ] {
        range_refusal(line, "degrees", "0 to 359");
    }
}

#[test]
fn rotate_refuses_a_pivot_that_is_not_two_whole_percents_and_names_the_range() {
    for line in [
        "rotate pip 90 pivot=101,50",
        "rotate pip 90 pivot=50,-1",
        "rotate pip 90 pivot=50",
        "rotate pip 90 pivot=a,b",
        "rotate pip 90 pivot=50,50,50",
        "rotate pip 90 pivot=,",
        "rotate pip 90 pivot=12.5,50",
    ] {
        range_refusal(line, "pivot", "0 to 100");
    }
}

#[test]
fn rotate_refuses_too_few_words_an_unknown_key_and_a_pivot_given_twice() {
    assert!(matches!(
        parse(&words("rotate pip")),
        Err(CommandError::Usage(usage)) if usage.starts_with("rotate <id> <degrees>")
    ));
    assert!(matches!(
        parse(&words("rotate")),
        Err(CommandError::Usage(_))
    ));
    assert!(matches!(
        parse(&words("rotate pip 90 speed=3")),
        Err(CommandError::BadChoice { .. })
    ));
    assert!(matches!(
        parse(&words("rotate pip 90 50,50")),
        Err(CommandError::BadPair(_))
    ));
    assert_eq!(
        parse(&words("rotate pip 90 pivot=1,1 pivot=2,2")),
        Err(CommandError::DuplicateKey("pivot".into()))
    );
}

#[test]
fn the_help_text_describes_rotate_with_its_range_and_pivot() {
    let help = usage_text();
    for text in ["rotate <id> <degrees> [pivot=X,Y]", "0 to 359", "pivot=X,Y"] {
        assert!(help.contains(text), "{text}");
    }
}

async fn run(running: &Running, line: &str) -> Result<String, DevError> {
    run_dev(running.port, &words(line)).await
}

async fn canvas(running: &Running) -> CanvasView {
    let body = get(running.port, "/canvas.json")
        .await
        .expect("answers")
        .body;
    serde_json::from_str(&body).expect("canvas")
}

async fn element(running: &Running, id: &str) -> ElementView {
    canvas(running)
        .await
        .elements
        .into_iter()
        .find(|element| element.id == id)
        .expect("exists")
}

async fn log_lines(running: &Running) -> Vec<String> {
    let events = get(running.port, "/events.json").await.expect("answers");
    let events: EventsReply = serde_json::from_str(&events.body).expect("events");
    events.events.into_iter().map(|line| line.text).collect()
}

#[tokio::test]
async fn rotating_through_the_running_host_turns_the_element_and_records_one_audit_line() {
    let running = start(&STARTER_FILES).await;
    run(&running, "spawn character pip name=Pip")
        .await
        .expect("spawns");
    let said = run(&running, "rotate pip 90 pivot=0,50")
        .await
        .expect("rotates");
    assert!(said.starts_with("ok (revision 2)"), "{said}");
    let turned = element(&running, "pip").await;
    assert_eq!(turned.rotation_degrees, 90);
    assert_eq!(
        turned.pivot,
        PivotView {
            x_percent: 0,
            y_percent: 50
        }
    );
    let lines = log_lines(&running).await;
    assert_eq!(
        lines
            .iter()
            .filter(|line| line.starts_with("CANVAS transform"))
            .collect::<Vec<_>>(),
        [
            "CANVAS transform element=pip kind=rotate degrees=90 pivot_x_percent=0 pivot_y_percent=50 revision=2"
        ]
    );
}

#[tokio::test]
async fn an_element_never_turned_reads_as_unturned_about_its_centre() {
    let running = start(&STARTER_FILES).await;
    run(&running, "spawn marker m1").await.expect("spawns");
    let marker = element(&running, "m1").await;
    assert_eq!(marker.rotation_degrees, 0);
    assert_eq!(marker.pivot, PivotView::default());
    assert_eq!((marker.pivot.x_percent, marker.pivot.y_percent), (50, 50));
}

#[tokio::test]
async fn a_second_rotate_without_a_pivot_keeps_the_hinge() {
    let running = start(&STARTER_FILES).await;
    run(&running, "spawn marker m1").await.expect("spawns");
    run(&running, "rotate m1 90 pivot=0,50")
        .await
        .expect("rotates");
    run(&running, "rotate m1 180").await.expect("rotates");
    let turned = element(&running, "m1").await;
    assert_eq!(turned.rotation_degrees, 180);
    assert_eq!(turned.pivot.x_percent, 0);
}

#[tokio::test]
async fn rotating_an_element_that_is_not_there_is_refused_and_recorded() {
    let running = start(&STARTER_FILES).await;
    let error = run(&running, "rotate ghost 90").await.expect_err("refused");
    assert!(
        matches!(
            &error,
            DevError::Rejected {
                error: ReplyError::Canvas {
                    cause: Some(RejectionCause::UnknownElement),
                    ..
                },
                ..
            }
        ),
        "{error:?}"
    );
    assert!(log_lines(&running).await.contains(
        &"CANVAS reject effect=transform cause=unknown_element element=ghost".to_owned()
    ));
}

async fn post_effect(running: &Running, body: &str) -> (StatusCode, Reply) {
    let response = post_json(running.port, "/dev/effect", body)
        .await
        .expect("answers");
    (
        response.status,
        serde_json::from_str(&response.body).expect("a reply"),
    )
}

#[tokio::test]
async fn the_wire_form_refuses_360_with_the_range_records_it_and_changes_nothing() {
    let running = start(&STARTER_FILES).await;
    run(&running, "spawn marker m1").await.expect("spawns");
    let (status, reply) = post_effect(
        &running,
        r#"{"op":"transform","id":"m1","rotation_degrees":360}"#,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(matches!(
        &reply,
        Reply::Rejected { message, .. } if message.contains("0 to 359")
    ));
    assert!(
        log_lines(&running).await.contains(
            &"CANVAS reject effect=transform cause=invalid_transform element=m1".to_owned()
        )
    );
    assert_eq!(element(&running, "m1").await.rotation_degrees, 0);
}

#[tokio::test]
async fn a_wire_request_with_a_good_rotation_and_a_bad_pivot_applies_neither() {
    let running = start(&STARTER_FILES).await;
    run(&running, "spawn marker m1").await.expect("spawns");
    let (status, reply) = post_effect(
        &running,
        r#"{"op":"transform","id":"m1","rotation_degrees":45,"pivot":{"x_percent":101,"y_percent":0}}"#,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(matches!(
        &reply,
        Reply::Rejected { message, .. } if message.contains("pivot") && message.contains("0 to 100")
    ));
    let unchanged = element(&running, "m1").await;
    assert_eq!(unchanged.rotation_degrees, 0);
    assert_eq!(unchanged.pivot, PivotView::default());
    assert!(
        !log_lines(&running)
            .await
            .iter()
            .any(|line| line.starts_with("CANVAS transform"))
    );
}

#[tokio::test]
async fn the_wire_form_accepts_the_edges_zero_and_359() {
    let running = start(&STARTER_FILES).await;
    run(&running, "spawn marker m1").await.expect("spawns");
    for degrees in [359, 0] {
        let body = format!(r#"{{"op":"transform","id":"m1","rotation_degrees":{degrees}}}"#);
        let (status, reply) = post_effect(&running, &body).await;
        assert_eq!(status, StatusCode::OK);
        assert!(matches!(reply, Reply::Done { .. }));
        assert_eq!(element(&running, "m1").await.rotation_degrees, degrees);
    }
}

#[tokio::test]
async fn a_rotation_wakes_a_waiting_page_and_the_frame_carries_it() {
    let running = start(&STARTER_FILES).await;
    run(&running, "spawn marker m1").await.expect("spawns");
    let frame = |body: String| serde_json::from_str::<Frame>(&body).expect("a frame");
    let before = frame(
        get(running.port, "/playground/frame")
            .await
            .expect("answers")
            .body,
    );
    run(&running, "rotate m1 270").await.expect("rotates");
    let after = frame(
        get(
            running.port,
            &format!("/playground/frame?after={}", before.frame),
        )
        .await
        .expect("answers")
        .body,
    );
    assert!(after.frame > before.frame);
    assert_eq!(after.canvas.elements[0].rotation_degrees, 270);
}

#[test]
fn an_element_view_writes_its_rotation_and_pivot_as_plain_numbers() {
    let json = serde_json::json!({
        "id": "m1", "kind": "marker", "x": 1.0, "y": 2.0, "label": "m1", "color": null,
        "shape": "circle", "width_units": 2.0, "height_units": 2.0, "fields": {},
        "problem": null, "travelling": null,
        "rotation_degrees": 90, "pivot": {"x_percent": 0, "y_percent": 50}
    });
    let view: ElementView = serde_json::from_value(json.clone()).expect("parses");
    assert_eq!(view.rotation_degrees, 90);
    assert_eq!(serde_json::to_value(&view).expect("writes"), json);
}

#[test]
fn an_element_from_a_frame_before_rotation_existed_still_parses_as_unturned() {
    let older = r#"{"id": "m1", "kind": "marker", "x": 1.0, "y": 2.0, "label": "m1",
        "color": null, "shape": "circle", "width_units": 2.0, "height_units": 2.0,
        "fields": {}, "problem": null}"#;
    let view: ElementView = serde_json::from_str(older).expect("parses");
    assert_eq!(view.rotation_degrees, 0);
    assert_eq!(view.pivot, PivotView::default());
}

#[tokio::test]
async fn a_repl_line_and_a_script_both_rotate() {
    let running = start(&STARTER_FILES).await;
    run(&running, "spawn marker m1").await.expect("spawns");
    assert!(matches!(
        run_line(running.port, "rotate m1 45 pivot=100,100").await,
        LineResult::Ok(text) if text.starts_with("ok")
    ));
    assert_eq!(element(&running, "m1").await.rotation_degrees, 45);
    assert!(matches!(
        run_line(running.port, "rotate m1 360").await,
        LineResult::Failed(text) if text.contains("0 to 359")
    ));

    let directory = tempfile::TempDir::new().expect("makes a temporary directory");
    let path = directory.path().join("turn.txt");
    std::fs::write(
        &path,
        "rotate m1 90\n# a note\nrotate m1 0\nwait 250\nrotate m1 180 pivot=0,0\n",
    )
    .expect("writes the script");
    let said = run(&running, &format!("script {}", path.display()))
        .await
        .expect("runs");
    assert!(said.ends_with("ran 4 commands, all ok"), "{said}");
    let turned = element(&running, "m1").await;
    assert_eq!(turned.rotation_degrees, 180);
    assert_eq!((turned.pivot.x_percent, turned.pivot.y_percent), (0, 0));
}

#[tokio::test]
async fn a_script_with_a_bad_rotation_on_a_late_line_runs_nothing() {
    let running = start(&STARTER_FILES).await;
    run(&running, "spawn marker m1").await.expect("spawns");
    let directory = tempfile::TempDir::new().expect("makes a temporary directory");
    let path = directory.path().join("turn.txt");
    std::fs::write(&path, "rotate m1 90\nrotate m1 360\n").expect("writes the script");
    let error = run(&running, &format!("script {}", path.display()))
        .await
        .expect_err("refused");
    assert!(error.to_string().contains("0 to 359"), "{error}");
    assert_eq!(element(&running, "m1").await.rotation_degrees, 0);
}

/// A transform request body that cannot be a whole number in range, and the range its refusal
/// must name.
async fn refused_with_the_range(running: &Running, body: &str, range: &str) {
    let (status, reply) = post_effect(running, body).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        matches!(&reply, Reply::Rejected { message, cause, .. }
            if message.contains(range) && *cause == Some(df_table::event::RejectionCause::InvalidTransform)),
        "{body}: {reply:?}"
    );
    assert!(
        log_lines(running).await.contains(
            &"CANVAS reject effect=transform cause=invalid_transform element=m1".to_owned()
        ),
        "{body}: recorded against the element, not as a malformed request"
    );
    assert_eq!(element(running, "m1").await.rotation_degrees, 0, "{body}");
}

#[tokio::test]
async fn a_rotation_too_big_for_64_bits_or_fractional_is_refused_with_its_range() {
    let running = start(&STARTER_FILES).await;
    run(&running, "spawn marker m1").await.expect("spawns");
    for number in ["18446744073709551615", "90.0", "9e1", "1e30", "-0.5", "-1"] {
        let body = format!(r#"{{"op":"transform","id":"m1","rotation_degrees":{number}}}"#);
        refused_with_the_range(&running, &body, "0 to 359").await;
    }
}

#[tokio::test]
async fn a_pivot_that_is_negative_fractional_or_huge_is_refused_with_its_range() {
    let running = start(&STARTER_FILES).await;
    run(&running, "spawn marker m1").await.expect("spawns");
    for pivot in ["-1", "50.5", "18446744073709551615", "1e3"] {
        let body = format!(
            r#"{{"op":"transform","id":"m1","rotation_degrees":0,"pivot":{{"x_percent":{pivot},"y_percent":50}}}}"#
        );
        refused_with_the_range(&running, &body, "0 to 100").await;
    }
}

#[tokio::test]
async fn a_missing_rotation_or_a_rotation_that_is_text_is_a_malformed_request() {
    let running = start(&STARTER_FILES).await;
    run(&running, "spawn marker m1").await.expect("spawns");
    for body in [
        r#"{"op":"transform","id":"m1"}"#,
        r#"{"op":"transform","id":"m1","rotation_degrees":"90"}"#,
        r#"{"op":"transform","id":"m1","rotation_degrees":null}"#,
    ] {
        let (status, reply) = post_effect(&running, body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(
            matches!(
                reply,
                Reply::Rejected {
                    cause: Some(df_table::event::RejectionCause::MalformedRequest),
                    ..
                }
            ),
            "{body}"
        );
    }
    assert_eq!(element(&running, "m1").await.rotation_degrees, 0);
}

#[tokio::test]
async fn a_pivot_change_on_a_turned_element_is_refused_once_and_the_log_says_why() {
    let running = start(&STARTER_FILES).await;
    run(&running, "spawn marker m1").await.expect("spawns");
    run(&running, "rotate m1 90").await.expect("rotates");
    let error = run(&running, "rotate m1 90 pivot=0,50")
        .await
        .expect_err("refused");
    assert!(
        matches!(
            &error,
            DevError::Rejected {
                error: ReplyError::Canvas {
                    cause: Some(RejectionCause::InvalidTransform),
                    ..
                },
                ..
            }
        ),
        "{error:?}"
    );
    let lines = log_lines(&running).await;
    assert_eq!(
        lines
            .iter()
            .filter(|line| line.starts_with("CANVAS reject effect=transform"))
            .collect::<Vec<_>>(),
        ["CANVAS reject effect=transform cause=invalid_transform element=m1"]
    );
    assert_eq!(element(&running, "m1").await.pivot, PivotView::default());
    run(&running, "rotate m1 0").await.expect("back to zero");
    tokio::time::sleep(crate::limits::TURN_SETTLE + std::time::Duration::from_millis(60)).await;
    run(&running, "rotate m1 0 pivot=0,50")
        .await
        .expect("now it may");
    let lines = log_lines(&running).await;
    assert!(
        lines
            .iter()
            .any(|line| line.contains("degrees=0 pivot_x_percent=0 pivot_y_percent=50")),
        "a pivot-only change shows in the log: {lines:?}"
    );
}
