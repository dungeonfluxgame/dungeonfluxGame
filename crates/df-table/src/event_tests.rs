use crate::element::{ElementId, FieldName, KindName};
use crate::event::{EffectKind, EventClass, RejectionCause, TableEvent};

fn id(text: &str) -> ElementId {
    ElementId::new(text).expect("valid id")
}

fn line(event: &TableEvent) -> String {
    event.audit_line().render()
}

#[test]
fn an_audit_line_is_the_area_then_a_verb_and_key_value_pairs() {
    let spawned = TableEvent::ElementSpawned {
        element: id("pip"),
        kind: KindName::new("character").expect("valid kind"),
        revision: 3,
    };
    assert_eq!(
        line(&spawned),
        "CANVAS spawn element=pip kind=character revision=3"
    );
    let changed = TableEvent::ElementChanged {
        element: id("pip"),
        field: FieldName::new("hit_points").expect("valid field"),
        revision: 4,
    };
    assert_eq!(
        line(&changed),
        "CANVAS set element=pip field=hit_points revision=4"
    );
    assert_eq!(
        line(&TableEvent::ElementMoved {
            element: id("pip"),
            revision: 5
        }),
        "CANVAS move element=pip revision=5"
    );
    assert_eq!(
        line(&TableEvent::ElementDespawned {
            element: id("pip"),
            revision: 6
        }),
        "CANVAS despawn element=pip revision=6"
    );
    assert_eq!(
        line(&TableEvent::CanvasCleared {
            removed: 2,
            revision: 7
        }),
        "CANVAS clear removed=2 revision=7"
    );
}

#[test]
fn a_rejection_names_the_effect_its_cause_and_the_element_when_known() {
    let full = TableEvent::EffectRejected {
        effect: Some(EffectKind::Spawn),
        element: Some(id("pip")),
        cause: RejectionCause::DuplicateElement,
    };
    assert_eq!(
        line(&full),
        "CANVAS reject effect=spawn cause=duplicate_element element=pip"
    );
    let unread = TableEvent::EffectRejected {
        effect: None,
        element: None,
        cause: RejectionCause::MalformedRequest,
    };
    assert_eq!(line(&unread), "CANVAS reject cause=malformed_request");
}

#[test]
fn reloads_and_rejected_reloads_are_distinct_verbs() {
    let reloaded = TableEvent::KindsReloaded {
        kinds: 4,
        invalid_elements: 1,
        revision: 9,
    };
    assert_eq!(
        line(&reloaded),
        "CANVAS reload kinds=4 invalid_elements=1 revision=9"
    );
    assert_eq!(
        line(&TableEvent::KindsRejected { errors: 2 }),
        "CANVAS reload_rejected errors=2"
    );
}

#[test]
fn only_moves_are_verbose_and_every_other_event_is_always_recorded() {
    let moved = TableEvent::ElementMoved {
        element: id("pip"),
        revision: 5,
    };
    assert_eq!(moved.class(), EventClass::Verbose);
    let cleared = TableEvent::CanvasCleared {
        removed: 2,
        revision: 6,
    };
    assert_eq!(cleared.class(), EventClass::Always);
    let rejected = TableEvent::KindsRejected { errors: 1 };
    assert_eq!(rejected.class(), EventClass::Always);
}

#[test]
fn every_rejection_cause_has_a_distinct_key() {
    let causes = [
        RejectionCause::UnknownKind,
        RejectionCause::DuplicateElement,
        RejectionCause::UnknownElement,
        RejectionCause::UnknownField,
        RejectionCause::InvalidValue,
        RejectionCause::InvalidName,
        RejectionCause::MissingRequiredField,
        RejectionCause::TooManyElements,
        RejectionCause::OutOfBounds,
        RejectionCause::MalformedRequest,
        RejectionCause::ForbiddenRequest,
    ];
    let keys: std::collections::BTreeSet<_> = causes.iter().map(|cause| cause.key()).collect();
    assert_eq!(keys.len(), causes.len());
}
