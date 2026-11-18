//! What the host sends for a frame and how the page asks for the next.

use crate::frame::{FrameReply, frame_address, parse_frame_reply};
use crate::overlay::Edit;

const FRAME: &str = r#"{
    "frame": 7,
    "canvas": {"revision": 3, "elements": []},
    "controls": {"revision": 2, "boot_id": "0001-1-0", "controls": {
        "fixture": null, "device": "both", "pace": "normal", "status_key": "fixture",
        "show_unavailable": false, "show_grid": false}},
    "asset_revision": "00000000000000aa.00000000000000bb"
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
fn a_frame_is_read_with_its_run_revision_canvas_and_file_revision() {
    let Ok(FrameReply::Frame(frame)) = parse_frame_reply(FRAME) else {
        panic!("a frame");
    };
    assert_eq!(frame.frame, 7);
    assert_eq!(frame.canvas.revision, 3);
    assert_eq!(frame.controls.boot_id, "0001-1-0");
    assert_eq!(frame.controls.revision, 2);
    assert_eq!(frame.asset_revision, "00000000000000aa.00000000000000bb");
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

#[test]
fn a_phone_is_sent_its_own_parts_and_none_of_the_shared_screen_or_the_developer_switches() {
    let Ok(FrameReply::Frame(frame)) = parse_frame_reply(FRAME) else {
        panic!("a frame");
    };
    let mut frame = *frame;
    let element = r#"{"id": "pip", "kind": "character", "x": 50, "y": 28, "label": "Pip",
        "color": null, "shape": "circle", "width_units": 9, "height_units": 9}"#;
    frame
        .canvas
        .elements
        .push(serde_json::from_str(element).expect("an element"));
    frame.controls.controls.show_grid = true;
    frame.controls.controls.stt_on = true;
    frame.controls.scene.backdrop = Edit::Clear;
    frame.controls.scene.clues = Edit::Clear;
    frame.controls.scene.suggested = Edit::Clear;
    frame.controls.scene.cue_count = 4;

    let player = frame.clone().for_player();
    assert_eq!(player.frame, frame.frame);
    assert_eq!(player.asset_revision, frame.asset_revision);
    assert_eq!(player.canvas.revision, 3);
    assert!(player.canvas.elements.is_empty(), "a phone draws no stage");
    assert!(!player.controls.controls.show_grid);
    assert!(
        player.controls.controls.stt_on,
        "the phone's own switch stays"
    );
    assert_eq!(player.controls.scene.backdrop, Edit::Keep);
    assert_eq!(
        player.controls.scene.clues,
        Edit::Clear,
        "the clue board is the room's: the phone mirrors it"
    );
    assert_eq!(
        player.controls.scene.suggested,
        Edit::Keep,
        "the developer's one phone is no seat's"
    );
    assert_eq!(player.controls.scene.cue_count, 4);
    assert_eq!(player.controls.boot_id, frame.controls.boot_id);
}

/// Every keep/clear/set slot of the scene overlay, by its JSON key.
const OVERLAY_SLOTS: [&str; 24] = [
    "caption",
    "party",
    "backdrop",
    "card",
    "camera",
    "narration",
    "seats",
    "brand",
    "lobby",
    "loading",
    "roleplay",
    "pills",
    "home",
    "suggested",
    "groups",
    "unavailable",
    "phone_image",
    "music",
    "clues",
    "story",
    "inventory",
    "roll",
    "shown_clue",
    "title_card",
];

/// The slots a phone is sent: the room's sound, and the pieces of the shared screen it mirrors.
const PLAYER_SLOTS: [&str; 8] = [
    "party",
    "seats",
    "card",
    "clues",
    "shown_clue",
    "title_card",
    "roll",
    "music",
];

#[test]
fn a_phone_is_sent_exactly_the_room_pieces_it_mirrors_and_no_other_slot() {
    let every_slot: serde_json::Map<String, serde_json::Value> = OVERLAY_SLOTS
        .iter()
        .map(|slot| ((*slot).to_owned(), serde_json::json!("clear")))
        .collect();
    let scene: crate::scene::SceneOverlay =
        serde_json::from_value(serde_json::Value::Object(every_slot))
            .expect("the list names every slot of the overlay");
    let Ok(FrameReply::Frame(frame)) = parse_frame_reply(FRAME) else {
        panic!("a frame");
    };
    let mut controls = frame.controls;
    controls.scene = scene;
    let sent = serde_json::to_value(crate::frame::player_controls(controls).scene)
        .expect("the overlay writes");
    let mut kept: Vec<&str> = sent
        .as_object()
        .expect("an object")
        .iter()
        .filter(|(_, value)| value.as_str() == Some("clear"))
        .map(|(key, _)| key.as_str())
        .collect();
    kept.sort_unstable();
    let mut expected = PLAYER_SLOTS.to_vec();
    expected.sort_unstable();
    assert_eq!(kept, expected);
}
