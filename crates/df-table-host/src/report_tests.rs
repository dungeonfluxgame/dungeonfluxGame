use df_table::kind::Shape;

use crate::event_log::LoggedLine;
use crate::report::{describe_done, format_events, format_kinds};
use crate::view::{CanvasView, FieldKind, FieldView, KindView};
use crate::wire::EventsReply;

fn view_with_a_kind() -> CanvasView {
    CanvasView {
        revision: 1,
        elements: Vec::new(),
        kinds: vec![KindView {
            name: "character".into(),
            description: "A person.".into(),
            shape: Shape::Circle,
            width_units: 6.0,
            height_units: 6.0,
            fields: vec![
                FieldView {
                    name: "name".into(),
                    kind: FieldKind::Text,
                    description: "text (at most 20)".into(),
                    required: true,
                    default: None,
                },
                FieldView {
                    name: "hit_points".into(),
                    kind: FieldKind::Number,
                    description: "number (0 to 999)".into(),
                    required: false,
                    default: Some(serde_json::json!(10.0)),
                },
            ],
        }],
    }
}

// A refused `Reply` prints as its `DevError::Rejected` (dev_tests).
#[test]
fn a_done_reply_reads_as_ok_with_its_revision_and_notices() {
    assert_eq!(
        describe_done(4, &["element 'pip': it no longer fits".to_owned()]),
        "ok (revision 4)\n  element 'pip': it no longer fits"
    );
}

#[test]
fn kinds_lists_each_field_with_its_description_a_star_for_required_and_its_default() {
    let text = format_kinds(&view_with_a_kind());
    assert!(text.contains("character (Circle 6x6) A person."));
    assert!(text.contains("  name*: text (at most 20)"));
    assert!(text.contains("  hit_points: number (0 to 999) default 10.0"));
}

#[test]
fn with_no_kinds_the_text_says_how_to_add_one() {
    let empty = CanvasView {
        revision: 0,
        elements: Vec::new(),
        kinds: Vec::new(),
    };
    assert!(format_kinds(&empty).contains("add a file to the kind directory"));
}

fn events(sequences: &[u64], latest: u64) -> EventsReply {
    EventsReply {
        events: sequences
            .iter()
            .map(|sequence| LoggedLine {
                sequence: *sequence,
                text: format!("CANVAS event {sequence}"),
            })
            .collect(),
        latest,
    }
}

#[test]
fn events_print_with_their_sequence_numbers_oldest_first() {
    let text = format_events(&events(&[1, 2], 2), None);
    assert_eq!(text, "   1  CANVAS event 1\n   2  CANVAS event 2");
}

#[test]
fn no_events_says_so() {
    assert_eq!(format_events(&events(&[], 0), Some(0)), "no events");
}

#[test]
fn a_reader_ahead_of_the_newest_sequence_is_told_the_host_restarted() {
    let text = format_events(&events(&[1], 1), Some(500));
    assert!(text.starts_with("(the host has restarted"));
    let normal = format_events(&events(&[2], 2), Some(1));
    assert!(!normal.contains("restarted"));
}

#[test]
fn the_playground_is_printed_one_setting_per_line_with_the_fixtures() {
    let reply = crate::wire::ControlsReply {
        revision: 3,
        boot_id: String::new(),
        controls: df_playground::controls::Controls {
            fixture: Some("a.json".into()),
            ..df_playground::controls::Controls::default()
        },
        scene: df_playground::scene::SceneOverlay::default(),
    };
    let text =
        crate::report::format_playground(&reply, &["a.json".to_owned(), "b.json".to_owned()]);
    assert_eq!(
        text,
        "fixture: a.json\ndevice: both\npace: normal\nstatus: fixture\nunavailable: false\ngrid: false\nsam: true\ntransition: fade 700 ms\nscene: as the fixture has it\nrevision: 3\nfixtures: a.json, b.json"
    );
    let empty = crate::report::format_playground(
        &crate::wire::ControlsReply {
            revision: 0,
            boot_id: String::new(),
            controls: df_playground::controls::Controls::default(),
            scene: df_playground::scene::SceneOverlay::default(),
        },
        &[],
    );
    assert!(empty.contains("fixture: (none)") && empty.contains("fixtures: (none)"));
}

#[test]
fn a_camera_only_move_is_reported_as_the_one_changed_slot() {
    let camera = df_playground::fixture::Camera {
        target: Some("door".to_owned()),
        zoom_percent: 150,
        duration_ms: 400,
        serial: 1,
        began_ms: 0,
        elapsed_ms: 0,
    };
    let reply = crate::wire::ControlsReply {
        revision: 1,
        boot_id: String::new(),
        controls: df_playground::controls::Controls::default(),
        scene: df_playground::scene::SceneOverlay {
            camera: df_playground::overlay::Edit::Set(camera),
            ..df_playground::scene::SceneOverlay::default()
        },
    };
    let text = crate::report::format_playground(&reply, &[]);
    assert!(
        text.lines().any(|line| line == "scene: changed: camera"),
        "{text}"
    );
}
