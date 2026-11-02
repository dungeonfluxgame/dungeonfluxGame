//! What the host sends for a frame and how the page asks for the next.

use crate::frame::{FrameReply, frame_address, parse_frame_reply};

const FRAME: &str = r#"{
    "frame": 7,
    "canvas": {"revision": 3, "elements": []},
    "controls": {"revision": 2, "boot_id": "0001-1-0", "controls": {
        "fixture": null, "device": "both", "pace": "normal", "status_key": "fixture",
        "show_unavailable": false, "show_grid": false}}
}"#;

#[test]
fn an_empty_answer_is_no_change_and_not_a_mistake() {
    for empty in ["", "  ", "\n"] {
        assert!(
            matches!(parse_frame_reply(empty), Ok(FrameReply::NoChange)),
            "{empty:?}"
        );
    }
}

#[test]
fn a_frame_is_read_with_its_run_revision_and_canvas() {
    let Ok(FrameReply::Frame(frame)) = parse_frame_reply(FRAME) else {
        panic!("a frame");
    };
    assert_eq!(frame.frame, 7);
    assert_eq!(frame.canvas.revision, 3);
    assert_eq!(frame.controls.boot_id, "0001-1-0");
    assert_eq!(frame.controls.revision, 2);
}

#[test]
fn an_answer_that_is_neither_empty_nor_a_frame_is_a_mistake_that_says_so() {
    for broken in ["{", "not json", "{\"frame\": 1}", "null"] {
        assert!(parse_frame_reply(broken).is_err(), "{broken:?}");
    }
}

#[test]
fn the_first_request_asks_for_what_is_there_and_later_ones_name_the_frame_and_the_run() {
    let base = "/playground/frame";
    assert_eq!(frame_address(base, None, Some("0001-1-0")), base);
    assert_eq!(
        frame_address(base, Some(5), None),
        "/playground/frame?after=5&idle=204"
    );
    assert_eq!(
        frame_address(base, Some(5), Some("0001-1-0")),
        "/playground/frame?after=5&boot=0001-1-0&idle=204"
    );
}

#[test]
fn a_run_name_that_is_not_plain_is_not_put_in_the_address() {
    let base = "/playground/frame";
    for hostile in ["", "a&b=c", "a b", "a/../b", "x?y"] {
        assert_eq!(
            frame_address(base, Some(1), Some(hostile)),
            "/playground/frame?after=1&idle=204",
            "{hostile:?}"
        );
    }
}
