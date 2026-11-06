use std::collections::BTreeSet;

use crate::effects::{
    ApplyOutcome, BeatName, ConversationOutcome, EffectCause, EffectRecord,
    TABLE_EFFECT_SCHEMA_VERSION, TableEffect, TableEffectKind, TableState, TableStateError,
};
use crate::identity::{MachineId, SceneId, SeatId, TableId, TableScope};
use crate::machine::slot::LoadCause;
use crate::testkit::correlation;

fn seat(number: u64) -> SeatId {
    SeatId::new(number)
}

fn scope() -> TableScope {
    TableScope::of_table(TableId::new(1))
}

fn beat() -> BeatName {
    BeatName::new("locked_gate").expect("valid beat")
}

fn machine(name: &str) -> MachineId {
    MachineId::new(name).expect("valid machine")
}

/// One effect of each kind. The match has no wildcard, so a new kind needs a sample here.
fn sample(kind: TableEffectKind) -> TableEffect {
    match kind {
        TableEffectKind::SeatJoined => TableEffect::SeatJoined { seat: seat(3) },
        TableEffectKind::SeatLeft => TableEffect::SeatLeft { seat: seat(1) },
        TableEffectKind::SceneEntered => TableEffect::SceneEntered {
            scene: SceneId::new(4),
        },
        TableEffectKind::BeatResolved => TableEffect::BeatResolved {
            scene: SceneId::new(4),
            beat: beat(),
        },
        TableEffectKind::ConversationEnded => TableEffect::ConversationEnded {
            scene: SceneId::new(4),
            outcome: ConversationOutcome::Moved,
        },
        TableEffectKind::HandOffRecorded => TableEffect::HandOffRecorded {
            from: machine("lobby"),
            to: machine("explore"),
            carried: 3,
        },
        TableEffectKind::SeatPresenceChanged => TableEffect::SeatPresenceChanged {
            seat: seat(1),
            presence: crate::table_state::Presence::Sleeping,
        },
        TableEffectKind::SeatMoved => TableEffect::SeatMoved {
            seat: seat(1),
            scene: SceneId::new(5),
        },
        TableEffectKind::TimeAdvanced => TableEffect::TimeAdvanced { advance_s: 6 },
        TableEffectKind::TablePaused => TableEffect::TablePaused,
        TableEffectKind::TableResumed => TableEffect::TableResumed,
        TableEffectKind::TableIdled => TableEffect::TableIdled,
        TableEffectKind::CastEntered => TableEffect::CastEntered {
            entry: crate::table_state_tests::creature("goblin", "Goblin", 4, 60),
        },
        TableEffectKind::CastLeft => TableEffect::CastLeft {
            cast: crate::table_state_tests::element("goblin"),
        },
        TableEffectKind::CastChanged => TableEffect::CastChanged {
            cast: crate::table_state_tests::element("goblin"),
            change: crate::table_state::CastChange::HitPoints { hit_points: 3 },
        },
        TableEffectKind::ActiveMachineSet => TableEffect::ActiveMachineSet {
            home: crate::table_state::MachineHome::Scene {
                scene: SceneId::new(4),
            },
            machine: machine("explore"),
            slot: crate::table_state_tests::slot_with("explore", "presenting"),
        },
        TableEffectKind::ActiveSlotUpdated => TableEffect::ActiveSlotUpdated {
            home: crate::table_state::MachineHome::Scene {
                scene: SceneId::new(4),
            },
            slot: crate::table_state_tests::slot_with("explore", "resolved"),
        },
        TableEffectKind::HandOffPushed => TableEffect::HandOffPushed {
            scene: SceneId::new(4),
            machine: machine("explore"),
            slot: crate::machine::slot::MachineSlot::from_parts(
                machine("explore"),
                1,
                serde_json::json!({"phase": "presenting"}),
            ),
        },
        TableEffectKind::HandOffPopped => TableEffect::HandOffPopped {
            scene: SceneId::new(4),
        },
    }
}

fn table() -> TableState {
    TableState::seated(3, &[seat(1), seat(2)]).expect("valid table")
}

fn apply(
    table: &mut TableState,
    turn: u64,
    effects: &[TableEffect],
) -> Result<ApplyOutcome, crate::effects::EffectError> {
    table.apply(&scope(), correlation(turn), effects)
}

#[test]
fn every_effect_kind_has_a_sample_a_distinct_key_and_a_version_this_engine_reads() {
    let keys: BTreeSet<_> = TableEffectKind::ALL.iter().map(|kind| kind.key()).collect();
    assert_eq!(keys.len(), TableEffectKind::ALL.len());
    for kind in TableEffectKind::ALL {
        assert_eq!(sample(*kind).kind(), *kind);
        assert!(kind.introduced_in() >= 1, "{}", kind.key());
        assert!(
            kind.introduced_in() <= TABLE_EFFECT_SCHEMA_VERSION,
            "{} claims a version newer than the engine",
            kind.key()
        );
    }
}

#[test]
fn every_effect_serializes_under_its_own_key_and_parses_back() {
    for kind in TableEffectKind::ALL {
        let effect = sample(*kind);
        let json = serde_json::to_value(&effect).expect("serializes");
        assert_eq!(json["effect"], kind.key());
        let parsed: TableEffect = serde_json::from_value(json).expect("parses");
        assert_eq!(parsed, effect);
    }
}

#[test]
fn an_effect_with_an_unknown_field_or_kind_does_not_parse() {
    let unknown_field = r#"{"effect":"seat_left","seat":1,"extra":2}"#;
    assert!(serde_json::from_str::<TableEffect>(unknown_field).is_err());
    let unknown_kind = r#"{"effect":"table_exploded"}"#;
    assert!(serde_json::from_str::<TableEffect>(unknown_kind).is_err());
}

#[test]
fn a_stamped_record_opens_and_a_newer_or_versionless_one_is_refused_with_its_fault() {
    let effect = sample(TableEffectKind::SeatJoined);
    let record = EffectRecord::stamp(&effect).expect("stamps");
    let json = serde_json::to_value(&record).expect("serializes");
    assert_eq!(json["schema_version"], TABLE_EFFECT_SCHEMA_VERSION);
    assert_eq!(record.open(), Ok(effect.clone()));
    for version in [TABLE_EFFECT_SCHEMA_VERSION + 1, 0] {
        let mut changed = json.clone();
        changed["schema_version"] = version.into();
        let record: EffectRecord = serde_json::from_value(changed).expect("parses");
        let refused = record.open().expect_err("refused");
        assert_eq!(refused.found, version);
        assert_eq!(refused.expected, TABLE_EFFECT_SCHEMA_VERSION);
        assert_eq!(refused.cause, LoadCause::UnsupportedVersion);
    }
}

#[test]
fn a_record_from_a_newer_engine_with_an_effect_this_one_does_not_know_is_a_version_refusal() {
    let text = r#"{"schema_version":2,"effect":{"effect":"quest_started","quest":"q1"}}"#;
    let record: EffectRecord = serde_json::from_str(text).expect("the record itself parses");
    let refused = record.open().expect_err("refused");
    assert_eq!(refused.cause, LoadCause::UnsupportedVersion);
    assert_eq!(
        (refused.found, refused.expected),
        (2, TABLE_EFFECT_SCHEMA_VERSION)
    );
    let event = refused.event(TableId::new(4));
    assert_eq!(
        event.render_audit_line(),
        "LOAD refused table=4 record=effect_intent version=2 expected=1 cause=unsupported_version"
    );
    assert!(refused.to_string().contains("unsupported_version"));
}

#[test]
fn a_record_at_a_known_version_that_holds_no_effect_is_malformed_not_a_serde_error() {
    for effect in [
        serde_json::json!({"effect": "quest_started", "quest": "q1"}),
        serde_json::json!({"effect": "seat_joined"}),
        serde_json::json!("seat_joined"),
        serde_json::json!(null),
    ] {
        let refused = EffectRecord::from_parts(TABLE_EFFECT_SCHEMA_VERSION, effect)
            .open()
            .expect_err("refused");
        assert_eq!(refused.cause, LoadCause::Malformed);
        assert_eq!(refused.found, TABLE_EFFECT_SCHEMA_VERSION);
    }
}

#[test]
fn an_effect_record_for_every_kind_round_trips_through_json() {
    for kind in TableEffectKind::ALL {
        let effect = sample(*kind);
        let record = EffectRecord::stamp(&effect).expect("stamps");
        let text = serde_json::to_string(&record).expect("serializes");
        let parsed: EffectRecord = serde_json::from_str(&text).expect("parses");
        assert_eq!(parsed.open(), Ok(effect));
    }
}

#[test]
fn a_commit_with_no_effects_is_remembered_and_changes_nothing() {
    let mut table = table();
    let before = table.clone();
    assert_eq!(
        apply(&mut table, 1, &[]),
        Ok(ApplyOutcome::Applied(Vec::new()))
    );
    assert!(table.same_world_as(&before));
    assert_eq!(apply(&mut table, 1, &[]), Ok(ApplyOutcome::AlreadyApplied));
}

#[test]
fn a_seat_joins_once_and_the_table_holds_only_its_capacity() {
    let mut table = table();
    let joined = apply(&mut table, 1, &[TableEffect::SeatJoined { seat: seat(3) }]);
    assert!(matches!(joined, Ok(ApplyOutcome::Applied(_))));
    assert!(table.seats().contains(&seat(3)));
    let twice = apply(&mut table, 2, &[TableEffect::SeatJoined { seat: seat(3) }]);
    assert_eq!(
        twice.expect_err("refused").cause,
        EffectCause::SeatAlreadySeated
    );
    let full = apply(&mut table, 3, &[TableEffect::SeatJoined { seat: seat(4) }]);
    assert_eq!(full.expect_err("refused").cause, EffectCause::TableFull);
    assert_eq!(table.seats().len(), 3);
}

#[test]
fn a_seat_that_is_not_seated_cannot_leave() {
    let mut table = table();
    let left = apply(&mut table, 1, &[TableEffect::SeatLeft { seat: seat(9) }]);
    let error = left.expect_err("refused");
    assert_eq!(error.cause, EffectCause::SeatNotSeated);
    assert_eq!(error.effect, TableEffectKind::SeatLeft);
}

#[test]
fn a_commit_with_a_refused_effect_applies_none_of_its_effects() {
    let mut table = table();
    let before = table.clone();
    let effects = [
        TableEffect::SeatJoined { seat: seat(3) },
        TableEffect::SceneEntered {
            scene: SceneId::new(4),
        },
        TableEffect::SeatLeft { seat: seat(9) },
    ];
    assert!(apply(&mut table, 1, &effects).is_err());
    assert_eq!(table, before);
}

#[test]
fn entering_a_scene_makes_it_active_and_clears_the_beat_and_a_beat_needs_its_scene_active() {
    let mut table = table();
    let scene = SceneId::new(4);
    let early = apply(
        &mut table,
        1,
        &[TableEffect::BeatResolved {
            scene,
            beat: beat(),
        }],
    );
    assert_eq!(
        early.expect_err("refused").cause,
        EffectCause::SceneNotActive
    );
    apply(&mut table, 2, &[TableEffect::SceneEntered { scene }]).expect("applies");
    assert_eq!(table.active_scene(), Some(scene));
    apply(
        &mut table,
        3,
        &[TableEffect::BeatResolved {
            scene,
            beat: beat(),
        }],
    )
    .expect("applies");
    assert_eq!(table.active_beat(), Some(&beat()));
    apply(
        &mut table,
        4,
        &[TableEffect::SceneEntered {
            scene: SceneId::new(5),
        }],
    )
    .expect("applies");
    assert_eq!(table.active_beat(), None);
}

#[test]
fn a_hand_off_is_recorded_without_changing_the_world() {
    let mut table = table();
    let before = table.clone();
    apply(&mut table, 1, &[sample(TableEffectKind::HandOffRecorded)]).expect("applies");
    assert!(table.same_world_as(&before));
    assert_ne!(table, before);
    let record = table.last_hand_off().expect("recorded");
    assert_eq!(record.from, machine("lobby"));
    assert_eq!(record.to, machine("explore"));
    assert_eq!(record.carried, 3);
}

#[test]
fn replaying_a_committed_operation_changes_nothing() {
    let mut table = table();
    let join = [TableEffect::SeatJoined { seat: seat(3) }];
    assert!(matches!(
        apply(&mut table, 1, &join),
        Ok(ApplyOutcome::Applied(_))
    ));
    let after_first = table.clone();
    assert_eq!(
        apply(&mut table, 1, &join),
        Ok(ApplyOutcome::AlreadyApplied)
    );
    assert_eq!(table, after_first);
}

#[test]
fn a_refused_commit_is_not_remembered_so_the_operation_can_be_sent_again() {
    let mut table = table();
    assert!(apply(&mut table, 1, &[TableEffect::SeatLeft { seat: seat(9) }]).is_err());
    let retried = apply(&mut table, 1, &[TableEffect::SeatLeft { seat: seat(1) }]);
    assert!(matches!(retried, Ok(ApplyOutcome::Applied(_))));
}

#[test]
fn the_oldest_applied_operation_is_forgotten_first_once_the_memory_is_full() {
    let mut table = TableState::new(8)
        .expect("valid table")
        .remembering_operations(2);
    for (turn, number) in [(1, 1), (2, 2), (3, 3)] {
        apply(
            &mut table,
            turn,
            &[TableEffect::SeatJoined { seat: seat(number) }],
        )
        .expect("applies");
    }
    let forgotten = apply(&mut table, 1, &[TableEffect::SeatLeft { seat: seat(1) }]);
    assert!(
        matches!(forgotten, Ok(ApplyOutcome::Applied(_))),
        "operation 1 was forgotten, so it is new again"
    );
    let remembered = apply(&mut table, 3, &[TableEffect::SeatLeft { seat: seat(3) }]);
    assert_eq!(remembered, Ok(ApplyOutcome::AlreadyApplied));
}

#[test]
fn a_memory_of_zero_remembers_no_operation() {
    let mut table = TableState::new(8)
        .expect("valid table")
        .remembering_operations(0);
    let join = [TableEffect::SeatJoined { seat: seat(1) }];
    apply(&mut table, 1, &join).expect("applies");
    assert!(
        apply(&mut table, 1, &join).is_err(),
        "the join runs again and the seat is taken"
    );
}

#[test]
fn each_applied_effect_is_one_class_a_event_with_the_pinned_context() {
    let mut table = table();
    let effects = [
        TableEffect::SeatJoined { seat: seat(3) },
        TableEffect::SceneEntered {
            scene: SceneId::new(4),
        },
    ];
    let Ok(ApplyOutcome::Applied(events)) = apply(&mut table, 7, &effects) else {
        panic!("the commit applies");
    };
    let lines: Vec<String> = events
        .iter()
        .map(|event| event.render_audit_line())
        .collect();
    assert_eq!(lines.len(), 2);
    assert!(
        lines[0].starts_with("EFFECT applied table=1 operation_id="),
        "{}",
        lines[0]
    );
    assert!(
        lines[0].ends_with("turn_seq=7 epoch=1 revision=7 effect=seat_joined"),
        "{}",
        lines[0]
    );
    assert!(lines[1].ends_with("effect=scene_entered"));
}

#[test]
fn a_refusal_event_names_the_effect_and_the_cause() {
    let mut table = table();
    let error =
        apply(&mut table, 7, &[TableEffect::SeatLeft { seat: seat(9) }]).expect_err("refused");
    let line = error.event(&scope(), correlation(7)).render_audit_line();
    assert!(line.starts_with("EFFECT refused table=1"), "{line}");
    assert!(
        line.ends_with("effect=seat_left cause=seat_not_seated"),
        "{line}"
    );
    assert!(error.to_string().contains("seat_left"));
}

#[test]
fn a_table_state_needs_a_real_capacity_and_a_seating_that_fits() {
    assert_eq!(
        TableState::new(0).err(),
        Some(TableStateError::ZeroCapacity)
    );
    assert_eq!(
        TableState::new(crate::limits::MAXIMUM_SEATS + 1).err(),
        Some(TableStateError::CapacityTooLarge)
    );
    assert_eq!(
        TableState::seated(1, &[seat(1), seat(2)]).err(),
        Some(TableStateError::TooManySeatsForCapacity)
    );
    assert_eq!(
        TableState::seated(3, &[seat(1), seat(1)]).err(),
        Some(TableStateError::DuplicateSeat(seat(1)))
    );
}
