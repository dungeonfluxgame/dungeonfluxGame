use std::collections::BTreeMap;

use crate::canvas::{Canvas, CanvasEffect, CanvasError, ElementProblem, MAXIMUM_ELEMENTS};
use crate::element::{
    CANVAS_HEIGHT_UNITS, CANVAS_WIDTH_UNITS, ElementId, FieldName, FieldValue, KindName, Position,
};
use crate::event::{EffectKind, RejectionCause, TableEvent};
use crate::field_type::ValueError;
use crate::testkit::{CHARACTER_KIND, MARKER_KIND, RecordingSink, registry_from};

fn canvas() -> Canvas {
    Canvas::new(registry_from(&[CHARACTER_KIND, MARKER_KIND]))
}

fn id(text: &str) -> ElementId {
    ElementId::new(text).expect("valid id")
}

fn kind(text: &str) -> KindName {
    KindName::new(text).expect("valid kind")
}

fn field(text: &str) -> FieldName {
    FieldName::new(text).expect("valid field")
}

fn spawn_effect(name: &str, display_name: &str, position: Position) -> CanvasEffect {
    CanvasEffect::Spawn {
        id: id(name),
        kind: kind("character"),
        position,
        fields: BTreeMap::from([(field("name"), FieldValue::Text(display_name.to_owned()))]),
    }
}

fn spawn_character(
    canvas: &mut Canvas,
    sink: &RecordingSink,
    name: &str,
) -> Result<(), CanvasError> {
    canvas.apply(spawn_effect(name, "Pip", Position::new(10.0, 10.0)), sink)
}

fn spawn_marker(canvas: &mut Canvas, sink: &RecordingSink, name: &str) -> Result<(), CanvasError> {
    canvas.apply(
        CanvasEffect::Spawn {
            id: id(name),
            kind: kind("marker"),
            position: Position::new(1.0, 1.0),
            fields: BTreeMap::new(),
        },
        sink,
    )
}

fn rejected(effect: EffectKind, element: Option<&str>, cause: RejectionCause) -> TableEvent {
    TableEvent::EffectRejected {
        effect: Some(effect),
        element: element.map(id),
        cause,
    }
}

#[test]
fn spawning_fills_the_defaults_and_records_one_event() {
    let mut canvas = canvas();
    let sink = RecordingSink::default();
    spawn_character(&mut canvas, &sink, "pip").expect("spawns");
    let element = canvas.element(&id("pip")).expect("is on the canvas");
    assert_eq!(
        element.fields.get(&field("hit_points")),
        Some(&FieldValue::Number(10.0))
    );
    assert!(element.fields.contains_key(&field("color")));
    assert_eq!(
        sink.events(),
        vec![TableEvent::ElementSpawned {
            element: id("pip"),
            kind: kind("character"),
            revision: 1
        }]
    );
}

#[test]
fn a_spawn_on_the_edges_of_the_canvas_works_and_just_outside_it_is_rejected() {
    let mut canvas = canvas();
    let sink = RecordingSink::default();
    for (name, x, y) in [
        ("a", 0.0, 0.0),
        ("b", CANVAS_WIDTH_UNITS, CANVAS_HEIGHT_UNITS),
    ] {
        canvas
            .apply(spawn_effect(name, "Edge", Position::new(x, y)), &sink)
            .expect("the edge is on the canvas");
    }
    for (x, y) in [
        (-0.01, 5.0),
        (CANVAS_WIDTH_UNITS + 0.01, 5.0),
        (5.0, CANVAS_HEIGHT_UNITS + 0.01),
        (f32::NAN, 5.0),
    ] {
        assert_eq!(
            canvas.apply(spawn_effect("c", "Out", Position::new(x, y)), &sink),
            Err(CanvasError::OutOfBounds)
        );
    }
    assert_eq!(canvas.len(), 2);
}

#[test]
fn spawning_with_a_value_that_does_not_fit_its_field_is_rejected() {
    let mut canvas = canvas();
    let sink = RecordingSink::default();
    let result = canvas.apply(
        CanvasEffect::Spawn {
            id: id("pip"),
            kind: kind("character"),
            position: Position::CENTER,
            fields: BTreeMap::from([
                (field("name"), FieldValue::Text("Pip".into())),
                (field("hit_points"), FieldValue::Number(5000.0)),
            ]),
        },
        &sink,
    );
    assert!(matches!(
        result,
        Err(CanvasError::InvalidValue {
            error: ValueError::OutOfRange { .. },
            ..
        })
    ));
    assert!(canvas.is_empty());
    assert_eq!(canvas.revision(), 0);
}

#[test]
fn spawning_an_unknown_kind_is_rejected_and_the_rejection_is_recorded() {
    let mut canvas = canvas();
    let sink = RecordingSink::default();
    let result = canvas.apply(
        CanvasEffect::Spawn {
            id: id("x"),
            kind: kind("dragon"),
            position: Position::CENTER,
            fields: BTreeMap::new(),
        },
        &sink,
    );
    assert_eq!(result, Err(CanvasError::UnknownKind(kind("dragon"))));
    assert_eq!(
        sink.events(),
        vec![rejected(
            EffectKind::Spawn,
            Some("x"),
            RejectionCause::UnknownKind
        )]
    );
    assert!(canvas.is_empty());
}

#[test]
fn a_second_element_with_the_same_id_is_rejected() {
    let mut canvas = canvas();
    let sink = RecordingSink::default();
    spawn_character(&mut canvas, &sink, "pip").expect("spawns");
    assert_eq!(
        spawn_character(&mut canvas, &sink, "pip"),
        Err(CanvasError::DuplicateElement(id("pip")))
    );
    assert_eq!(canvas.len(), 1);
}

#[test]
fn a_required_field_with_no_default_must_be_given() {
    let mut canvas = canvas();
    let sink = RecordingSink::default();
    let result = canvas.apply(
        CanvasEffect::Spawn {
            id: id("pip"),
            kind: kind("character"),
            position: Position::CENTER,
            fields: BTreeMap::new(),
        },
        &sink,
    );
    assert_eq!(
        result,
        Err(CanvasError::MissingRequiredField(field("name")))
    );
}

#[test]
fn a_field_the_kind_does_not_declare_is_rejected() {
    let mut canvas = canvas();
    let sink = RecordingSink::default();
    let result = canvas.apply(
        CanvasEffect::Spawn {
            id: id("m"),
            kind: kind("marker"),
            position: Position::CENTER,
            fields: BTreeMap::from([(field("name"), FieldValue::Text("x".into()))]),
        },
        &sink,
    );
    assert!(matches!(result, Err(CanvasError::UnknownField { .. })));
}

#[test]
fn setting_a_field_checks_its_type_and_range_and_records_the_change() {
    let mut canvas = canvas();
    let sink = RecordingSink::default();
    spawn_character(&mut canvas, &sink, "pip").expect("spawns");
    let set = |canvas: &mut Canvas, name: &str, value: FieldValue| {
        canvas.apply(
            CanvasEffect::Set {
                id: id("pip"),
                field: field(name),
                value,
            },
            &sink,
        )
    };
    assert_eq!(
        set(&mut canvas, "hit_points", FieldValue::Number(7.0)),
        Ok(())
    );
    assert!(matches!(
        set(&mut canvas, "hit_points", FieldValue::Number(5000.0)),
        Err(CanvasError::InvalidValue { .. })
    ));
    assert!(matches!(
        set(&mut canvas, "hit_points", FieldValue::Text("seven".into())),
        Err(CanvasError::InvalidValue { .. })
    ));
    assert!(matches!(
        set(&mut canvas, "nickname", FieldValue::Text("x".into())),
        Err(CanvasError::UnknownField { .. })
    ));
    assert_eq!(
        canvas
            .element(&id("pip"))
            .and_then(|element| element.fields.get(&field("hit_points"))),
        Some(&FieldValue::Number(7.0))
    );
    assert!(sink.events().contains(&TableEvent::ElementChanged {
        element: id("pip"),
        field: field("hit_points"),
        revision: 2
    }));
}

#[test]
fn setting_a_field_of_an_element_that_is_not_there_is_rejected() {
    let mut canvas = canvas();
    let sink = RecordingSink::default();
    let result = canvas.apply(
        CanvasEffect::Set {
            id: id("ghost"),
            field: field("name"),
            value: FieldValue::Text("x".into()),
        },
        &sink,
    );
    assert_eq!(result, Err(CanvasError::UnknownElement(id("ghost"))));
}

#[test]
fn moving_to_the_edges_works_and_outside_the_canvas_or_to_nowhere_is_rejected() {
    let mut canvas = canvas();
    let sink = RecordingSink::default();
    spawn_character(&mut canvas, &sink, "pip").expect("spawns");
    let move_to = |canvas: &mut Canvas, name: &str, x: f32, y: f32| {
        canvas.apply(
            CanvasEffect::Move {
                id: id(name),
                position: Position::new(x, y),
            },
            &sink,
        )
    };
    assert_eq!(
        move_to(&mut canvas, "pip", 101.0, 5.0),
        Err(CanvasError::OutOfBounds)
    );
    assert_eq!(
        move_to(&mut canvas, "pip", f32::NAN, 5.0),
        Err(CanvasError::OutOfBounds)
    );
    assert_eq!(
        move_to(&mut canvas, "nobody", 1.0, 1.0),
        Err(CanvasError::UnknownElement(id("nobody")))
    );
    assert_eq!(move_to(&mut canvas, "pip", CANVAS_WIDTH_UNITS, 0.0), Ok(()));
    assert_eq!(
        canvas.element(&id("pip")).map(|element| element.position),
        Some(Position::new(CANVAS_WIDTH_UNITS, 0.0))
    );
}

#[test]
fn despawn_removes_an_element_and_clear_removes_all_of_them() {
    let mut canvas = canvas();
    let sink = RecordingSink::default();
    spawn_character(&mut canvas, &sink, "pip").expect("spawns");
    spawn_character(&mut canvas, &sink, "mara").expect("spawns");
    assert_eq!(
        canvas.apply(CanvasEffect::Despawn { id: id("pip") }, &sink),
        Ok(())
    );
    assert_eq!(
        canvas.apply(CanvasEffect::Despawn { id: id("pip") }, &sink),
        Err(CanvasError::UnknownElement(id("pip")))
    );
    assert_eq!(canvas.apply(CanvasEffect::Clear, &sink), Ok(()));
    assert!(canvas.is_empty());
    assert!(sink.events().contains(&TableEvent::CanvasCleared {
        removed: 1,
        revision: 4
    }));
}

#[test]
fn every_change_advances_the_revision_by_one_and_a_rejection_leaves_it_alone() {
    let mut canvas = canvas();
    let sink = RecordingSink::default();
    spawn_character(&mut canvas, &sink, "pip").expect("spawns");
    assert_eq!(canvas.revision(), 1);
    canvas
        .apply(
            CanvasEffect::Set {
                id: id("pip"),
                field: field("hit_points"),
                value: FieldValue::Number(3.0),
            },
            &sink,
        )
        .expect("sets");
    assert_eq!(canvas.revision(), 2);
    canvas
        .apply(
            CanvasEffect::Move {
                id: id("pip"),
                position: Position::new(5.0, 5.0),
            },
            &sink,
        )
        .expect("moves");
    assert_eq!(canvas.revision(), 3);
    let _ = spawn_character(&mut canvas, &sink, "pip");
    assert_eq!(canvas.revision(), 3);
    canvas
        .apply(CanvasEffect::Despawn { id: id("pip") }, &sink)
        .expect("despawns");
    assert_eq!(canvas.revision(), 4);
}

#[test]
fn each_effect_records_exactly_the_event_that_describes_it() {
    let mut canvas = canvas();
    let sink = RecordingSink::default();
    spawn_character(&mut canvas, &sink, "pip").expect("spawns");
    let _ = spawn_character(&mut canvas, &sink, "pip");
    let _ = canvas.apply(
        CanvasEffect::Move {
            id: id("nobody"),
            position: Position::CENTER,
        },
        &sink,
    );
    canvas
        .apply(CanvasEffect::Despawn { id: id("pip") }, &sink)
        .expect("despawns");
    canvas.apply(CanvasEffect::Clear, &sink).expect("clears");
    assert_eq!(
        sink.events(),
        vec![
            TableEvent::ElementSpawned {
                element: id("pip"),
                kind: kind("character"),
                revision: 1
            },
            rejected(
                EffectKind::Spawn,
                Some("pip"),
                RejectionCause::DuplicateElement
            ),
            rejected(
                EffectKind::Move,
                Some("nobody"),
                RejectionCause::UnknownElement
            ),
            TableEvent::ElementDespawned {
                element: id("pip"),
                revision: 2
            },
            TableEvent::CanvasCleared {
                removed: 0,
                revision: 3
            },
        ]
    );
}

#[test]
fn the_canvas_rejects_an_element_beyond_the_maximum() {
    let mut canvas = canvas();
    let sink = RecordingSink::default();
    for index in 0..MAXIMUM_ELEMENTS {
        spawn_marker(&mut canvas, &sink, &format!("m{index}")).expect("room for it");
    }
    assert_eq!(
        spawn_marker(&mut canvas, &sink, "extra"),
        Err(CanvasError::TooManyElements)
    );
}

/// The character kind of the testkit with one more field line added at the end of its fields.
fn character_kind_with(extra_field: &str, hit_point_maximum: u32) -> String {
    let default = hit_point_maximum.min(10);
    format!(
        r##"{{"name": "character", "shape": "circle", "width": 6, "height": 6,
            "fields": [
                {{"name": "name", "type": "text", "maximum_length": 20, "required": true}},
                {{"name": "color", "type": "color", "default": "#4f8cff"}},
                {{"name": "hit_points", "type": "number", "minimum": 0, "maximum": {hit_point_maximum}, "default": {default}}},
                {{"name": "role", "type": "choice", "choices": ["hero", "npc", "foe"], "default": "hero"}},
                {{"name": "hidden", "type": "flag", "default": false}}{extra_field}
            ]}}"##
    )
}

#[test]
fn reloading_a_kind_with_a_new_required_field_flags_the_elements_made_before_it() {
    let mut canvas = canvas();
    let sink = RecordingSink::default();
    spawn_character(&mut canvas, &sink, "pip").expect("spawns");
    let stricter = registry_from(&[&character_kind_with(
        r#", {"name": "class", "type": "text", "required": true}"#,
        999,
    )]);
    let report = canvas.replace_registry(stricter, &sink);
    assert_eq!(report.kinds, 1);
    assert_eq!(
        report.problems,
        vec![ElementProblem {
            element: id("pip"),
            problem: CanvasError::RequiredFieldAbsent(field("class"))
        }]
    );
    assert!(canvas.element(&id("pip")).is_some());
    assert_eq!(canvas.problems().len(), 1);
    assert_eq!(
        sink.events().last(),
        Some(&TableEvent::KindsReloaded {
            kinds: 1,
            invalid_elements: 1,
            revision: 2
        })
    );
}

#[test]
fn tightening_a_limit_flags_an_element_that_now_breaks_it() {
    let mut canvas = canvas();
    let sink = RecordingSink::default();
    spawn_character(&mut canvas, &sink, "pip").expect("spawns");
    let report = canvas.replace_registry(registry_from(&[&character_kind_with("", 5)]), &sink);
    assert!(matches!(
        report.problems[0].problem,
        CanvasError::InvalidValue {
            error: ValueError::OutOfRange { .. },
            ..
        }
    ));
}

#[test]
fn dropping_a_field_from_a_kind_flags_the_elements_that_still_carry_it() {
    let mut canvas = canvas();
    let sink = RecordingSink::default();
    spawn_character(&mut canvas, &sink, "pip").expect("spawns");
    let slimmer = registry_from(&[
        r#"{"name": "character", "shape": "circle", "width": 6, "height": 6,
            "fields": [{"name": "name", "type": "text", "required": true}]}"#,
    ]);
    let report = canvas.replace_registry(slimmer, &sink);
    assert!(matches!(
        report.problems[0].problem,
        CanvasError::UnknownField { .. }
    ));
}

#[test]
fn removing_a_kind_flags_its_elements_but_keeps_them() {
    let mut canvas = canvas();
    let sink = RecordingSink::default();
    spawn_character(&mut canvas, &sink, "pip").expect("spawns");
    let report = canvas.replace_registry(registry_from(&[MARKER_KIND]), &sink);
    assert_eq!(
        report.problems[0].problem,
        CanvasError::UnknownKind(kind("character"))
    );
    assert_eq!(canvas.len(), 1);
}

#[test]
fn a_canvas_error_says_what_is_wrong_in_plain_words() {
    assert_eq!(
        CanvasError::UnknownElement(id("ghost")).to_string(),
        "there is no element 'ghost'"
    );
    assert_eq!(
        CanvasError::DuplicateElement(id("pip")).to_string(),
        "'pip' already exists"
    );
    assert!(CanvasError::OutOfBounds.to_string().contains("56.25"));
}
