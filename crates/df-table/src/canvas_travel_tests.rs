//! Travel through the canvas: the effects, the tick and the events they record.

use std::collections::BTreeMap;

use crate::canvas::{Canvas, CanvasEffect, CanvasError};
use crate::element::{ElementId, FieldName, FieldValue, KindName, Position};
use crate::event::{EffectKind, RejectionCause, TableEvent};
use crate::limits::MAXIMUM_SPEED;
use crate::motion::Speed;
use crate::testkit::{CHARACTER_KIND, RecordingSink, registry_from};

fn id(text: &str) -> ElementId {
    ElementId::new(text).expect("valid id")
}

fn canvas_with_pip(sink: &RecordingSink) -> Canvas {
    let mut canvas = Canvas::new(registry_from(&[CHARACTER_KIND]));
    canvas
        .apply(
            CanvasEffect::Spawn {
                id: id("pip"),
                kind: KindName::new("character").expect("valid"),
                position: Position::new(10.0, 10.0),
                fields: BTreeMap::from([(
                    FieldName::new("name").expect("valid"),
                    FieldValue::Text("Pip".into()),
                )]),
            },
            sink,
        )
        .expect("spawns");
    canvas
}

fn travel(
    canvas: &mut Canvas,
    sink: &RecordingSink,
    x: f32,
    y: f32,
    speed: f32,
) -> Result<(), CanvasError> {
    canvas.apply(
        CanvasEffect::Travel {
            id: id("pip"),
            to: Position::new(x, y),
            speed: Speed::new(speed).expect("valid speed"),
        },
        sink,
    )
}

fn position(canvas: &Canvas) -> Position {
    canvas.element(&id("pip")).expect("exists").position
}

#[test]
fn travel_starts_a_journey_records_it_and_changes_nothing_else_yet() {
    let sink = RecordingSink::default();
    let mut canvas = canvas_with_pip(&sink);
    travel(&mut canvas, &sink, 50.0, 10.0, 20.0).expect("starts");
    assert_eq!(position(&canvas), Position::new(10.0, 10.0));
    assert!(canvas.is_moving());
    assert!(canvas.element(&id("pip")).expect("exists").motion.is_some());
    assert!(sink.events().contains(&TableEvent::ElementTravelling {
        element: id("pip"),
        revision: 2
    }));
}

#[test]
fn ticks_carry_the_element_to_its_target_and_record_only_the_arrival() {
    let sink = RecordingSink::default();
    let mut canvas = canvas_with_pip(&sink);
    travel(&mut canvas, &sink, 20.0, 10.0, 10.0).expect("starts");
    let before = sink.events().len();
    let mut arrived_after = None;
    for tick in 1..=20 {
        let report = canvas.tick(0.1, &sink);
        if !report.arrived.is_empty() {
            arrived_after = Some(tick);
            assert_eq!(report.arrived, vec![id("pip")]);
            break;
        }
        assert_eq!(report.moved, 1);
    }
    assert_eq!(
        arrived_after,
        Some(10),
        "ten units at ten per second is one second"
    );
    assert_eq!(position(&canvas), Position::new(20.0, 10.0));
    assert!(!canvas.is_moving());
    let recorded: Vec<TableEvent> = sink.events().split_off(before);
    assert_eq!(
        recorded.len(),
        1,
        "only the arrival is an event, not each step"
    );
    assert!(
        matches!(&recorded[0], TableEvent::ElementArrived { element, .. } if *element == id("pip"))
    );
}

#[test]
fn a_tick_that_moves_something_is_one_revision_and_an_idle_tick_is_none() {
    let sink = RecordingSink::default();
    let mut canvas = canvas_with_pip(&sink);
    let idle = canvas.revision();
    assert_eq!(canvas.tick(0.1, &sink).moved, 0);
    assert_eq!(canvas.revision(), idle);
    travel(&mut canvas, &sink, 90.0, 10.0, 10.0).expect("starts");
    let started = canvas.revision();
    canvas.tick(0.1, &sink);
    canvas.tick(0.1, &sink);
    assert_eq!(canvas.revision(), started + 2);
}

#[test]
fn a_move_ends_a_journey_and_a_new_travel_replaces_the_old_one() {
    let sink = RecordingSink::default();
    let mut canvas = canvas_with_pip(&sink);
    travel(&mut canvas, &sink, 90.0, 10.0, 10.0).expect("starts");
    canvas
        .apply(
            CanvasEffect::Move {
                id: id("pip"),
                position: Position::new(5.0, 5.0),
            },
            &sink,
        )
        .expect("moves");
    assert!(!canvas.is_moving());
    canvas.tick(0.1, &sink);
    assert_eq!(position(&canvas), Position::new(5.0, 5.0));

    travel(&mut canvas, &sink, 6.0, 5.0, 10.0).expect("starts");
    travel(&mut canvas, &sink, 5.0, 6.0, 10.0).expect("replaces");
    for _ in 0..3 {
        canvas.tick(0.1, &sink);
    }
    assert_eq!(position(&canvas), Position::new(5.0, 6.0));
}

#[test]
fn stop_ends_a_journey_where_the_element_stands_and_is_recorded() {
    let sink = RecordingSink::default();
    let mut canvas = canvas_with_pip(&sink);
    travel(&mut canvas, &sink, 90.0, 10.0, 10.0).expect("starts");
    canvas.tick(0.1, &sink);
    let stopped_at = position(&canvas);
    canvas
        .apply(CanvasEffect::Stop { id: id("pip") }, &sink)
        .expect("stops");
    canvas.tick(0.1, &sink);
    assert_eq!(position(&canvas), stopped_at);
    assert!(sink.events().iter().any(
        |event| matches!(event, TableEvent::ElementStopped { element, .. } if *element == id("pip"))
    ));
}

#[test]
fn stopping_an_element_that_is_not_travelling_is_allowed_and_a_missing_one_is_refused() {
    let sink = RecordingSink::default();
    let mut canvas = canvas_with_pip(&sink);
    assert!(
        canvas
            .apply(CanvasEffect::Stop { id: id("pip") }, &sink)
            .is_ok()
    );
    assert_eq!(
        canvas.apply(CanvasEffect::Stop { id: id("ghost") }, &sink),
        Err(CanvasError::UnknownElement(id("ghost")))
    );
}

#[test]
fn travel_to_a_point_off_the_canvas_or_for_a_missing_element_is_refused_and_recorded() {
    let sink = RecordingSink::default();
    let mut canvas = canvas_with_pip(&sink);
    assert_eq!(
        travel(&mut canvas, &sink, 500.0, 10.0, 10.0),
        Err(CanvasError::OutOfBounds)
    );
    let ghost = canvas.apply(
        CanvasEffect::Travel {
            id: id("ghost"),
            to: Position::CENTER,
            speed: Speed::new(10.0).expect("valid"),
        },
        &sink,
    );
    assert_eq!(ghost, Err(CanvasError::UnknownElement(id("ghost"))));
    assert!(!canvas.is_moving());
    let rejections: Vec<_> = sink
        .events()
        .into_iter()
        .filter_map(|event| match event {
            TableEvent::EffectRejected { effect, cause, .. } => Some((effect, cause)),
            _ => None,
        })
        .collect();
    assert_eq!(
        rejections,
        vec![
            (Some(EffectKind::Travel), RejectionCause::OutOfBounds),
            (Some(EffectKind::Travel), RejectionCause::UnknownElement)
        ]
    );
}

#[test]
fn removing_a_travelling_element_ends_its_journey_with_it() {
    let sink = RecordingSink::default();
    let mut canvas = canvas_with_pip(&sink);
    travel(&mut canvas, &sink, 90.0, 10.0, 10.0).expect("starts");
    canvas
        .apply(CanvasEffect::Despawn { id: id("pip") }, &sink)
        .expect("despawns");
    assert!(!canvas.is_moving());
    assert_eq!(canvas.tick(0.1, &sink).moved, 0);
}

#[test]
fn clearing_ends_every_journey() {
    let sink = RecordingSink::default();
    let mut canvas = canvas_with_pip(&sink);
    travel(&mut canvas, &sink, 90.0, 10.0, 10.0).expect("starts");
    canvas.apply(CanvasEffect::Clear, &sink).expect("clears");
    assert!(!canvas.is_moving() && canvas.is_empty());
}

#[test]
fn the_fastest_speed_still_lands_exactly_on_the_target() {
    let sink = RecordingSink::default();
    let mut canvas = canvas_with_pip(&sink);
    travel(&mut canvas, &sink, 99.0, 55.0, MAXIMUM_SPEED).expect("starts");
    for _ in 0..4 {
        canvas.tick(0.1, &sink);
    }
    assert_eq!(position(&canvas), Position::new(99.0, 55.0));
}

#[test]
fn the_travel_events_render_as_audit_lines() {
    let line = |event: TableEvent| event.render_audit_line();
    assert_eq!(
        line(TableEvent::ElementTravelling {
            element: id("pip"),
            revision: 3
        }),
        "CANVAS travel element=pip revision=3"
    );
    assert_eq!(
        line(TableEvent::ElementArrived {
            element: id("pip"),
            revision: 4
        }),
        "CANVAS arrive element=pip revision=4"
    );
    assert_eq!(
        line(TableEvent::ElementStopped {
            element: id("pip"),
            revision: 5
        }),
        "CANVAS stop element=pip revision=5"
    );
    assert_eq!(RejectionCause::InvalidSpeed.key(), "invalid_speed");
}
