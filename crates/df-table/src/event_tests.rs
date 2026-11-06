use crate::config::{SettingCause, SettingName};
use crate::element::{Degrees, ElementId, FieldName, KindName, Percent};
use crate::event::{
    AUDIT_LINE_CAPACITY_BYTES, EffectKind, EventClass, FailureCause, PipelineStep, RejectionCause,
    TableEvent, TransformKind,
};
use crate::identity::{
    Correlation, EvidenceRef, MachineId, OperationId, RecoveryEpoch, SceneId, SessionRevision,
    TableId, TableScope, TurnSeq,
};
use crate::outcome::{DeclineCode, OutcomeKind};
use crate::ports::PortName;
use crate::private_data::LengthClass;

fn id(text: &str) -> ElementId {
    ElementId::new(text).expect("valid id")
}

fn line(event: &TableEvent) -> String {
    event.render_audit_line()
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
fn a_transform_line_names_the_element_the_kind_the_degrees_and_the_revision() {
    let turned = TableEvent::ElementTransformed {
        element: id("pip"),
        kind: TransformKind::Rotate,
        degrees: Degrees::new(90).expect("valid degrees"),
        pivot_x_percent: Percent::new(0).expect("valid percent"),
        pivot_y_percent: Percent::new(50).expect("valid percent"),
        revision: 7,
    };
    assert_eq!(
        line(&turned),
        "CANVAS transform element=pip kind=rotate degrees=90 pivot_x_percent=0 pivot_y_percent=50 revision=7"
    );
    assert_eq!(turned.class(), EventClass::Always);
    assert_eq!(turned.area(), "CANVAS");
}

#[test]
fn a_rejected_transform_names_the_effect_and_an_out_of_range_cause() {
    let refused = TableEvent::EffectRejected {
        effect: Some(EffectKind::Transform),
        element: Some(id("pip")),
        cause: RejectionCause::InvalidTransform,
    };
    assert_eq!(
        line(&refused),
        "CANVAS reject effect=transform cause=invalid_transform element=pip"
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
        RejectionCause::InvalidTransform,
        RejectionCause::MalformedRequest,
        RejectionCause::ForbiddenRequest,
    ];
    let keys: std::collections::BTreeSet<_> = causes.iter().map(|cause| cause.key()).collect();
    assert_eq!(keys.len(), causes.len());
}

fn scope() -> TableScope {
    TableScope::of_table(TableId::new(1))
        .in_scene(SceneId::new(2))
        .in_machine(MachineId::new("explore").expect("valid machine"))
}

fn correlation() -> Correlation {
    Correlation {
        operation_id: OperationId::from_hex("0000000000000000000000000000feed")
            .expect("valid operation id"),
        turn_seq: TurnSeq::new(7),
        revision: SessionRevision::new(RecoveryEpoch::FIRST, 9),
    }
}

fn engine_events() -> Vec<TableEvent> {
    vec![
        TableEvent::InputReceived {
            scope: TableScope::of_table(TableId::new(1)),
            correlation: correlation(),
            length: LengthClass::Medium,
            evidence: Some(
                EvidenceRef::from_hex("00000000000000000000000000000e01").expect("valid reference"),
            ),
        },
        TableEvent::InputDeclined {
            scope: scope(),
            correlation: correlation(),
            code: DeclineCode::NotYourTurn,
        },
        TableEvent::StepFinished {
            scope: scope(),
            correlation: correlation(),
            step: PipelineStep::Check,
            outcome: Some(OutcomeKind::Applied),
            ms: 12,
        },
        TableEvent::TurnApplied {
            scope: scope(),
            correlation: correlation(),
            ms: 340,
        },
        TableEvent::BoundaryFailed {
            scope: scope(),
            correlation: correlation(),
            port: PortName::Narrator,
            cause: FailureCause::TimedOut,
            ms: 8000,
        },
        TableEvent::SettingRejected {
            name: SettingName::new("narration_late_s").expect("valid setting name"),
            cause: SettingCause::OutOfRange,
        },
        TableEvent::SettingKeysIgnored { count: 2 },
    ]
}

fn canvas_events() -> Vec<TableEvent> {
    vec![
        TableEvent::ElementSpawned {
            element: id("pip"),
            kind: KindName::new("character").expect("valid kind"),
            revision: 3,
        },
        TableEvent::ElementDespawned {
            element: id("pip"),
            revision: 4,
        },
        TableEvent::ElementChanged {
            element: id("pip"),
            field: FieldName::new("hit_points").expect("valid field"),
            revision: 5,
        },
        TableEvent::ElementMoved {
            element: id("pip"),
            revision: 6,
        },
        TableEvent::ElementTravelling {
            element: id("pip"),
            revision: 7,
        },
        TableEvent::ElementArrived {
            element: id("pip"),
            revision: 8,
        },
        TableEvent::ElementStopped {
            element: id("pip"),
            revision: 9,
        },
        TableEvent::CanvasCleared {
            removed: 2,
            revision: 10,
        },
        TableEvent::EffectRejected {
            effect: Some(EffectKind::Travel),
            element: Some(id("pip")),
            cause: RejectionCause::InvalidSpeed,
        },
        TableEvent::EffectRejected {
            effect: None,
            element: None,
            cause: RejectionCause::MalformedRequest,
        },
        TableEvent::KindsReloaded {
            kinds: 4,
            invalid_elements: 1,
            revision: 11,
        },
        TableEvent::KindsRejected { errors: 2 },
        TableEvent::ElementTransformed {
            element: id("pip"),
            kind: TransformKind::Rotate,
            degrees: Degrees::new(359).expect("valid degrees"),
            pivot_x_percent: Percent::new(50).expect("valid percent"),
            pivot_y_percent: Percent::new(50).expect("valid percent"),
            revision: 12,
        },
    ]
}

fn keys_of(rendered: &str) -> Vec<&str> {
    rendered
        .split(' ')
        .skip(2)
        .filter_map(|pair| pair.split_once('=').map(|(key, _)| key))
        .collect()
}

#[test]
fn engine_events_render_one_line_with_the_pinned_field_names_in_a_fixed_order() {
    let expected = [
        "INPUT received table=1 operation_id=0000000000000000000000000000feed turn_seq=7 \
         epoch=1 revision=9 length=medium evidence=00000000000000000000000000000e01",
        "INPUT declined table=1 scene=2 machine=explore \
         operation_id=0000000000000000000000000000feed turn_seq=7 epoch=1 revision=9 \
         code=not_your_turn",
        "STEP finished table=1 scene=2 machine=explore \
         operation_id=0000000000000000000000000000feed turn_seq=7 epoch=1 revision=9 \
         step=check outcome=applied ms=12",
        "TURN applied table=1 scene=2 machine=explore \
         operation_id=0000000000000000000000000000feed turn_seq=7 epoch=1 revision=9 ms=340",
        "BOUNDARY failed table=1 scene=2 machine=explore \
         operation_id=0000000000000000000000000000feed turn_seq=7 epoch=1 revision=9 \
         port=narrator cause=timed_out ms=8000",
        "SETTING rejected name=narration_late_s cause=out_of_range",
        "SETTING keys_ignored count=2",
    ];
    let lines: Vec<String> = engine_events().iter().map(line).collect();
    assert_eq!(lines, expected);
}

#[test]
fn the_class_a_field_names_of_question_89_are_pinned() {
    let context = [
        "table",
        "scene",
        "machine",
        "operation_id",
        "turn_seq",
        "epoch",
        "revision",
    ];
    let with = |extra: &[&'static str]| [context.as_slice(), extra].concat();
    let events = engine_events();
    assert_eq!(
        keys_of(&line(&events[0])),
        [
            "table",
            "operation_id",
            "turn_seq",
            "epoch",
            "revision",
            "length",
            "evidence"
        ]
    );
    assert_eq!(keys_of(&line(&events[1])), with(&["code"]));
    assert_eq!(keys_of(&line(&events[2])), with(&["step", "outcome", "ms"]));
    assert_eq!(keys_of(&line(&events[3])), with(&["ms"]));
    assert_eq!(keys_of(&line(&events[4])), with(&["port", "cause", "ms"]));
}

#[test]
fn a_step_that_passes_the_input_on_has_no_outcome_key() {
    let passed = TableEvent::StepFinished {
        scope: scope(),
        correlation: correlation(),
        step: PipelineStep::Admit,
        outcome: None,
        ms: 1,
    };
    assert!(!line(&passed).contains("outcome="));
    assert!(line(&passed).ends_with("step=admit ms=1"));
}

#[test]
fn only_a_move_and_a_pipeline_step_are_class_b_and_every_other_event_is_class_a() {
    for event in canvas_events().into_iter().chain(engine_events()) {
        let is_detail = matches!(
            event,
            TableEvent::ElementMoved { .. } | TableEvent::StepFinished { .. }
        );
        let expected = if is_detail {
            EventClass::Verbose
        } else {
            EventClass::Always
        };
        assert_eq!(event.class(), expected, "{}", line(&event));
    }
}

#[test]
fn every_engine_event_files_under_its_own_audit_area() {
    let areas: Vec<_> = engine_events().iter().map(TableEvent::area).collect();
    assert_eq!(
        areas,
        [
            "INPUT", "INPUT", "STEP", "TURN", "BOUNDARY", "SETTING", "SETTING"
        ]
    );
}

#[test]
fn every_event_serializes_and_parses_back_to_itself() {
    for event in canvas_events().into_iter().chain(engine_events()) {
        let json = serde_json::to_string(&event).expect("an event serializes");
        let parsed: TableEvent = serde_json::from_str(&json).expect("an event parses");
        assert_eq!(parsed, event, "{json}");
        assert_eq!(parsed.render_audit_line(), event.render_audit_line());
    }
}

#[test]
fn a_serialized_event_names_its_variant_and_keeps_the_record_shape() {
    let declined = &engine_events()[1];
    let json = serde_json::to_value(declined).expect("an event serializes to a value");
    assert_eq!(json["event"], "input_declined");
    assert_eq!(json["code"], "not_your_turn");
    assert_eq!(json["scope"]["table"], 1);
    assert_eq!(json["scope"]["machine"], "explore");
    assert_eq!(
        json["correlation"]["operation_id"],
        "0000000000000000000000000000feed"
    );
    assert_eq!(json["correlation"]["turn_seq"], 7);
}

#[test]
fn an_event_with_an_unknown_field_or_variant_or_bad_id_does_not_parse() {
    let unknown_field = r#"{"event":"kinds_rejected","errors":1,"extra":2}"#;
    assert!(serde_json::from_str::<TableEvent>(unknown_field).is_err());
    let unknown_variant = r#"{"event":"table_exploded"}"#;
    assert!(serde_json::from_str::<TableEvent>(unknown_variant).is_err());
    let bad_operation_id = r#"{"event":"input_declined",
        "scope":{"table":1,"scene":null,"machine":null},
        "correlation":{"operation_id":"zz","turn_seq":1,"revision":{"epoch":1,"sequence":1}},
        "code":"not_viable"}"#;
    assert!(serde_json::from_str::<TableEvent>(bad_operation_id).is_err());
}

const STACK_BUFFER_BYTES: usize = 512;

struct StackWriter {
    buffer: [u8; STACK_BUFFER_BYTES],
    length: usize,
}

impl std::fmt::Write for StackWriter {
    fn write_str(&mut self, text: &str) -> std::fmt::Result {
        let end = self.length + text.len();
        let target = self
            .buffer
            .get_mut(self.length..end)
            .ok_or(std::fmt::Error)?;
        target.copy_from_slice(text.as_bytes());
        self.length = end;
        Ok(())
    }
}

#[test]
fn a_line_renders_into_a_fixed_stack_buffer_with_no_string_involved() {
    for event in canvas_events().into_iter().chain(engine_events()) {
        let mut writer = StackWriter {
            buffer: [0; STACK_BUFFER_BYTES],
            length: 0,
        };
        event
            .write_audit_line(&mut writer)
            .expect("every line fits the buffer");
        let written = std::str::from_utf8(&writer.buffer[..writer.length]).expect("ascii");
        assert_eq!(written, event.render_audit_line());
    }
}

#[test]
fn an_input_received_line_carries_a_length_class_and_no_text() {
    let without_evidence = TableEvent::InputReceived {
        scope: TableScope::of_table(TableId::new(1)),
        correlation: correlation(),
        length: LengthClass::Short,
        evidence: None,
    };
    let rendered = line(&without_evidence);
    assert!(rendered.ends_with("revision=9 length=short"), "{rendered}");
    assert!(!rendered.contains("evidence="));
    let json = serde_json::to_value(&without_evidence).expect("serializes");
    assert_eq!(json["length"], "short");
    assert_eq!(json["evidence"], serde_json::Value::Null);
}

fn widest_ids() -> (TableScope, Correlation) {
    let machine = MachineId::new(&"m".repeat(crate::limits::MAXIMUM_NAME_LENGTH))
        .expect("a name of the maximum length is valid");
    let scope = TableScope::of_table(TableId::new(u64::MAX))
        .in_scene(SceneId::new(u64::MAX))
        .in_machine(machine);
    let epoch = RecoveryEpoch::new(u64::MAX).expect("a large epoch is valid");
    let correlation = Correlation {
        operation_id: OperationId::from_bytes(&[0xff; 16]).expect("valid id"),
        turn_seq: TurnSeq::new(u64::MAX),
        revision: SessionRevision::new(epoch, u64::MAX),
    };
    (scope, correlation)
}

#[test]
fn the_widest_possible_line_of_every_event_fits_the_reserved_capacity() {
    let (scope, correlation) = widest_ids();
    let longest_name = SettingName::new(&"s".repeat(crate::limits::MAXIMUM_NAME_LENGTH))
        .expect("a name of the maximum length is valid");
    let long = |letter: &str| letter.repeat(crate::limits::MAXIMUM_NAME_LENGTH);
    let from = crate::machine::StateName::new(&long("f")).expect("a maximum-length state name");
    let to = crate::machine::StateName::new(&long("t")).expect("a maximum-length state name");
    let target = crate::identity::MachineId::new(&long("x")).expect("a maximum-length machine");
    let widest = [
        TableEvent::MachineTransitioned {
            scope: scope.clone(),
            correlation,
            from,
            to,
        },
        TableEvent::MachineExited {
            scope: scope.clone(),
            correlation,
            kind: crate::machine::HandOffKind::ReturnToFrameBelow,
            target: Some(target),
            carried: usize::MAX,
        },
        TableEvent::InputReceived {
            scope: scope.clone(),
            correlation,
            length: LengthClass::Medium,
            evidence: Some(EvidenceRef::from_bytes(&[0xff; 16]).expect("valid reference")),
        },
        TableEvent::InputDeclined {
            scope: scope.clone(),
            correlation,
            code: DeclineCode::CastingRestricted,
        },
        TableEvent::StepFinished {
            scope: scope.clone(),
            correlation,
            step: PipelineStep::Interpret,
            outcome: Some(OutcomeKind::NeedsClarification),
            ms: u64::MAX,
        },
        TableEvent::TurnApplied {
            scope: scope.clone(),
            correlation,
            ms: u64::MAX,
        },
        TableEvent::BoundaryFailed {
            scope,
            correlation,
            port: PortName::KnowledgeView,
            cause: FailureCause::InvalidAnswer,
            ms: u64::MAX,
        },
        TableEvent::SettingRejected {
            name: longest_name,
            cause: SettingCause::InconsistentWithRelatedSetting,
        },
    ];
    for event in widest {
        let rendered = event.render_audit_line();
        assert!(
            rendered.len() <= AUDIT_LINE_CAPACITY_BYTES,
            "{} bytes: {rendered}",
            rendered.len()
        );
    }
}

#[test]
fn rendering_reserves_the_capacity_once_and_the_line_never_regrows_it() {
    for event in canvas_events().into_iter().chain(engine_events()) {
        let rendered = event.render_audit_line();
        assert!(rendered.len() <= AUDIT_LINE_CAPACITY_BYTES);
        assert!(rendered.capacity() >= AUDIT_LINE_CAPACITY_BYTES);
        assert!(rendered.capacity() < 2 * AUDIT_LINE_CAPACITY_BYTES);
    }
}

fn owner_queue_events() -> Vec<TableEvent> {
    let scope = TableScope::of_table(TableId::new(1));
    vec![
        TableEvent::InputRefused {
            scope: scope.clone(),
            correlation: correlation(),
            cause: crate::owner_queue::NotDeclinedCause::Duplicate,
        },
        TableEvent::InputServed {
            scope: scope.clone(),
            correlation: correlation(),
            lane: crate::owner_queue::Lane::SeatControls,
            ms: 12,
        },
        TableEvent::InputAbandoned {
            scope: scope.clone(),
            correlation: correlation(),
            cause: FailureCause::TimedOut,
            ms: 3_600_000,
        },
        TableEvent::InputFinishedLate {
            scope,
            correlation: correlation(),
            outcome: OutcomeKind::Applied,
        },
    ]
}

/// Catches an owner-queue event whose line drops or reorders one of the pinned context keys, or
/// whose own keys change (question 89: class A fields only grow).
#[test]
fn the_owner_queue_events_render_the_pinned_context_then_their_own_keys() {
    let context =
        "table=1 operation_id=0000000000000000000000000000feed turn_seq=7 epoch=1 revision=9";
    let expected = [
        format!("INPUT refused {context} cause=duplicate"),
        format!("INPUT served {context} lane=seat_controls ms=12"),
        format!("INPUT abandoned {context} cause=timed_out ms=3600000"),
        format!("INPUT finished_late {context} outcome=applied"),
    ];
    let rendered: Vec<String> = owner_queue_events().iter().map(line).collect();
    assert_eq!(rendered, expected);
}

/// Catches a class change: a refusal, a serve, an abandon and a late finish are all state an
/// operator reads, so none may be sampled away.
#[test]
fn the_owner_queue_events_are_class_a_in_the_input_area_and_round_trip_through_serde() {
    for event in owner_queue_events() {
        assert_eq!(event.class(), EventClass::Always);
        assert_eq!(event.area(), "INPUT");
        let json = serde_json::to_string(&event).expect("serializes");
        let back: TableEvent = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(back, event);
    }
}

/// Catches a field wide enough to push an owner-queue line past the reserved capacity.
#[test]
fn the_widest_owner_queue_event_lines_fit_the_reserved_capacity() {
    let correlation = Correlation {
        operation_id: OperationId::from_bytes(&[0xff; 16]).expect("valid id"),
        turn_seq: TurnSeq::new(u64::MAX),
        revision: SessionRevision::new(RecoveryEpoch::new(u64::MAX).expect("valid"), u64::MAX),
    };
    let scope = TableScope::of_table(TableId::new(u64::MAX));
    let widest = [
        TableEvent::InputAbandoned {
            scope: scope.clone(),
            correlation,
            cause: FailureCause::InvalidAnswer,
            ms: u64::MAX,
        },
        TableEvent::InputFinishedLate {
            scope,
            correlation,
            outcome: OutcomeKind::NeedsClarification,
        },
    ];
    for event in widest {
        let rendered = event.render_audit_line();
        assert!(
            rendered.len() <= AUDIT_LINE_CAPACITY_BYTES,
            "{} bytes: {rendered}",
            rendered.len()
        );
    }
}
