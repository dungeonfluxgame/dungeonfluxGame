use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::effects::{EffectCause, TableEffectKind};
use crate::element::NameError;
use crate::event::{AUDIT_LINE_CAPACITY_BYTES, EventClass, TableEvent};
use crate::identity::{Correlation, MachineId, OperationId, SceneId, SeatId, TableId, TableScope};
use crate::identity::{RecoveryEpoch, SessionRevision, TurnSeq};
use crate::limits::MAXIMUM_CARRY_ITEMS;
use crate::machine::slot::{ErrorClass, LoadCause, LoadError, MachineSlot, RecordKind, SlotError};
use crate::machine::{
    CarryItem, Entry, HandOff, HandOffError, HandOffKind, HandOffTarget, InputClaim, MachineError,
    MachineFailureCause, MachineReadout, MachineSpec, ReadoutError, RequestKey, Scope, SpecError,
    StateName, StepOutcome, StepResult, SuspendPolicy, entered_event, exited_event, step_events,
};
use crate::outcome::{Decline, DeclineCode, DeclineError, Outcome};
use crate::ports::{ActionId, PortName};
use crate::testkit::correlation;

fn spec() -> MachineSpec {
    MachineSpec::new(
        "sample",
        Scope::Group,
        2,
        &["first", "second"],
        SuspendPolicy::FinishAndHold,
    )
    .expect("valid spec")
}

fn state(name: &str) -> StateName {
    StateName::new(name).expect("valid state name")
}

fn action(name: &str) -> ActionId {
    ActionId::new(name).expect("valid action")
}

fn scope() -> TableScope {
    TableScope::of_table(TableId::new(1))
        .in_scene(SceneId::new(2))
        .in_machine(MachineId::new("sample").expect("valid id"))
}

#[test]
fn a_spec_is_built_only_valid() {
    let new = |id: &str, version: u32, states: &[&str]| {
        MachineSpec::new(
            id,
            Scope::Group,
            version,
            states,
            SuspendPolicy::FinishAndHold,
        )
    };
    assert_eq!(
        new("Bad Name", 1, &["a"]).err(),
        Some(SpecError::InvalidName(NameError::MustStartWithLetter))
    );
    assert_eq!(
        new("ok", 1, &["bad state"]).err(),
        Some(SpecError::InvalidName(NameError::InvalidCharacter(' ')))
    );
    assert_eq!(
        new("ok", 0, &["a"]).err(),
        Some(SpecError::ZeroSchemaVersion)
    );
    assert_eq!(new("ok", 1, &[]).err(), Some(SpecError::NoStates));
    assert_eq!(
        new("ok", 1, &["a", "a"]).err(),
        Some(SpecError::DuplicateState(state("a")))
    );
}

#[test]
fn a_spec_refuses_a_duplicate_target_or_action_and_dedupes_a_port_and_a_claim() {
    let investigate = action("investigate");
    let with_target = spec().handing_off_to("lobby").expect("first target");
    assert_eq!(
        with_target.clone().handing_off_to("lobby").err(),
        Some(SpecError::DuplicateHandOffTarget(
            MachineId::new("lobby").expect("valid id")
        ))
    );
    assert!(matches!(
        with_target.handing_off_to("Not Valid").err(),
        Some(SpecError::InvalidName(_))
    ));
    let with_action = spec()
        .with_rules_action(investigate.clone())
        .expect("first action");
    assert_eq!(
        with_action.with_rules_action(investigate.clone()).err(),
        Some(SpecError::DuplicateRulesAction(investigate.clone()))
    );
    let seatless = spec()
        .with_unseated_action(investigate.clone())
        .expect("first action");
    assert_eq!(
        seatless.unseated_actions(),
        std::slice::from_ref(&investigate)
    );
    assert_eq!(
        seatless.with_unseated_action(investigate.clone()).err(),
        Some(SpecError::DuplicateUnseatedAction(investigate))
    );
    let ported = spec()
        .requiring_port(PortName::Broker)
        .requiring_port(PortName::Broker);
    assert_eq!(ported.required_ports(), [PortName::Broker]);
    let claiming = spec()
        .claiming(InputClaim::Timer)
        .claiming(InputClaim::Timer);
    assert_eq!(claiming.claimed_inputs(), [InputClaim::Timer]);
    assert!(!spec().reads_entry());
    assert!(spec().reading_entry().reads_entry());
}

#[test]
fn every_spec_error_reads_as_a_sentence() {
    let errors = [
        SpecError::InvalidName(NameError::Empty),
        SpecError::NoStates,
        SpecError::DuplicateState(state("a")),
        SpecError::ZeroSchemaVersion,
        SpecError::DuplicateHandOffTarget(MachineId::new("lobby").expect("valid id")),
        SpecError::DuplicateRulesAction(action("look")),
        SpecError::DuplicateUnseatedAction(action("join")),
    ];
    for error in errors {
        assert!(error.to_string().len() > 10, "{error:?}");
    }
}

#[test]
fn the_scope_rules_let_a_table_start_a_scene_and_keep_a_seat_lane_in_its_lane() {
    let allowed = [
        (Scope::Table, Scope::Group),
        (Scope::Group, Scope::Group),
        (Scope::Group, Scope::Table),
        (Scope::Seat, Scope::Seat),
    ];
    for from in Scope::ALL {
        for to in Scope::ALL {
            assert_eq!(
                from.may_hand_off_to(*to),
                allowed.contains(&(*from, *to)),
                "{} to {}",
                from.key(),
                to.key()
            );
        }
    }
}

#[test]
fn scopes_suspend_policies_and_claims_have_distinct_keys() {
    let scopes: BTreeSet<_> = Scope::ALL.iter().map(|scope| scope.key()).collect();
    assert_eq!(scopes.len(), Scope::ALL.len());
    let policies: BTreeSet<_> = SuspendPolicy::ALL
        .iter()
        .map(|policy| policy.key())
        .collect();
    assert_eq!(policies.len(), SuspendPolicy::ALL.len());
    let claims: BTreeSet<_> = InputClaim::ALL.iter().map(|claim| claim.key()).collect();
    assert_eq!(claims.len(), InputClaim::ALL.len());
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Sample {
    count: u32,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
enum Tagged {
    First { count: u32 },
    Second,
}

#[test]
fn a_state_is_saved_under_the_machines_id_and_version_and_read_back() {
    let slot = MachineSlot::encode(&spec(), &Sample { count: 5 }).expect("encodes");
    assert_eq!(slot.machine().as_str(), "sample");
    assert_eq!(slot.schema_version(), 2);
    assert_eq!(slot.payload(), &json!({ "count": 5 }));
    assert_eq!(slot.decode::<Sample>(&spec()), Ok(Sample { count: 5 }));
}

#[test]
fn a_record_for_another_machine_or_version_or_with_a_damaged_state_does_not_decode() {
    let id = MachineId::new("sample").expect("valid id");
    let other = MachineId::new("other").expect("valid id");
    let wrong_machine = MachineSlot::from_parts(other.clone(), 2, json!({ "count": 5 }));
    assert_eq!(
        wrong_machine.decode::<Sample>(&spec()),
        Err(LoadError::WrongMachine {
            expected: id.clone(),
            found: other
        })
    );
    let old = MachineSlot::from_parts(id.clone(), 1, json!({ "count": 5 }));
    assert_eq!(
        old.decode::<Sample>(&spec()),
        Err(LoadError::UnsupportedVersion {
            found: 1,
            expected: 2
        })
    );
    let damaged = MachineSlot::from_parts(id, 2, json!({ "count": "many" }));
    assert_eq!(
        damaged.decode::<Sample>(&spec()),
        Err(LoadError::Malformed {
            version: 2,
            class: ErrorClass::Data
        })
    );
}

#[test]
fn a_decode_failure_keeps_its_error_class_and_never_the_records_contents() {
    let id = MachineId::new("sample").expect("valid id");
    let secret = "SENTINEL-SECRET-7f3a";
    let damaged = MachineSlot::from_parts(id, 2, json!({ "count": secret }));
    let error = damaged.decode::<Sample>(&spec()).expect_err("damaged");
    assert!(matches!(
        error,
        LoadError::Malformed {
            class: ErrorClass::Data,
            ..
        }
    ));
    assert!(!error.to_string().contains(secret));
    assert!(!format!("{error:?}").contains(secret));
    let classes: BTreeSet<_> = ErrorClass::ALL.iter().map(|class| class.key()).collect();
    assert_eq!(classes.len(), ErrorClass::ALL.len());
    assert_eq!(
        ErrorClass::from(serde_json::error::Category::Syntax),
        ErrorClass::Syntax
    );
    assert_eq!(
        ErrorClass::from(serde_json::error::Category::Eof),
        ErrorClass::Eof
    );
    assert_eq!(
        ErrorClass::from(serde_json::error::Category::Io),
        ErrorClass::Io
    );
}

#[test]
fn the_state_name_is_read_from_the_tag_alone() {
    let id = MachineId::new("sample").expect("valid id");
    let tagged = |payload| MachineSlot::from_parts(id.clone(), 2, payload);
    let named = |slot: &MachineSlot| slot.named_state(&spec(), "phase");
    assert_eq!(
        named(&tagged(json!({ "phase": "first" }))),
        Ok(state("first"))
    );
    assert_eq!(
        named(&tagged(
            json!({ "phase": "first", "count": "not a number" })
        )),
        Ok(state("first")),
        "the rest of the record is not read"
    );
    let malformed = LoadError::Malformed {
        version: 2,
        class: ErrorClass::Data,
    };
    for payload in [
        json!({ "phase": "third" }),
        json!({ "phase": 3 }),
        json!({ "phase": "Not A Name" }),
        json!({}),
        json!("first"),
        json!(null),
    ] {
        assert_eq!(
            named(&tagged(payload.clone())),
            Err(malformed.clone()),
            "{payload}"
        );
    }
    let other = MachineId::new("other").expect("valid id");
    let foreign = MachineSlot::from_parts(other, 2, json!({ "phase": "first" }));
    assert!(matches!(
        named(&foreign),
        Err(LoadError::WrongMachine { .. })
    ));
    let old = MachineSlot::from_parts(id, 1, json!({ "phase": "first" }));
    assert!(matches!(
        named(&old),
        Err(LoadError::UnsupportedVersion { .. })
    ));
    let round_trip = MachineSlot::encode(&spec(), &Tagged::First { count: 1 }).expect("encodes");
    assert_eq!(named(&round_trip), Ok(state("first")));
    let unit = MachineSlot::encode(&spec(), &Tagged::Second).expect("encodes");
    assert_eq!(named(&unit), Ok(state("second")));
}

#[test]
fn a_saved_record_round_trips_through_json_and_rejects_unknown_fields() {
    let slot = MachineSlot::encode(&spec(), &Sample { count: 5 }).expect("encodes");
    let json = serde_json::to_string(&slot).expect("serializes");
    assert_eq!(
        serde_json::from_str::<MachineSlot>(&json).expect("parses"),
        slot
    );
    let extra = r#"{"machine":"sample","schema_version":2,"state":{},"extra":1}"#;
    assert!(serde_json::from_str::<MachineSlot>(extra).is_err());
}

#[test]
fn every_load_fault_has_a_distinct_key_and_a_refusal_event_names_the_table_the_record_and_both_versions()
 {
    let keys: BTreeSet<_> = LoadCause::ALL.iter().map(|fault| fault.key()).collect();
    assert_eq!(keys.len(), LoadCause::ALL.len());
    let kinds: BTreeSet<_> = RecordKind::ALL.iter().map(|kind| kind.key()).collect();
    assert_eq!(kinds.len(), RecordKind::ALL.len());
    let record = MachineSlot::from_parts(MachineId::new("sample").expect("valid id"), 9, json!({}));
    let error = LoadError::UnsupportedVersion {
        found: 9,
        expected: 2,
    };
    let event = error.refusal_event(TableId::new(3), &spec(), &record);
    assert_eq!(
        event.render_audit_line(),
        "LOAD refused table=3 record=machine_slot machine=sample version=9 expected=2 \
         cause=unsupported_version"
    );
    assert_eq!(event.class(), EventClass::Always);
    assert_eq!(error.cause(), LoadCause::UnsupportedVersion);
    assert!(error.to_string().contains('9'));
}

#[test]
fn a_hand_off_carries_a_typed_list_up_to_a_named_bound() {
    let target = MachineId::new("explore").expect("valid id");
    let carry = vec![
        CarryItem::Scene(SceneId::new(1)),
        CarryItem::Seat(SeatId::new(2)),
    ];
    let hand_off = HandOff::new(target.clone(), carry.clone()).expect("builds");
    assert_eq!(hand_off.machine_target(), Some(&target));
    assert_eq!(hand_off.target(), &HandOffTarget::Machine(target.clone()));
    assert_eq!(hand_off.carry(), carry);
    let too_many = vec![CarryItem::Seat(SeatId::new(1)); MAXIMUM_CARRY_ITEMS + 1];
    assert_eq!(
        HandOff::new(target, too_many).err(),
        Some(HandOffError::TooManyCarryItems)
    );
}

#[test]
fn a_hand_off_can_release_a_lane_or_return_to_the_frame_below_and_then_names_no_machine() {
    let release = HandOff::to(HandOffTarget::ReleaseLane, Vec::new()).expect("builds");
    let below = HandOff::to(HandOffTarget::ReturnToFrameBelow, Vec::new()).expect("builds");
    assert_eq!(release.machine_target(), None);
    assert_eq!(below.machine_target(), None);
    assert_eq!(release.target().kind(), HandOffKind::ReleaseLane);
    assert_eq!(below.target().kind(), HandOffKind::ReturnToFrameBelow);
    let keys: BTreeSet<_> = HandOffKind::ALL.iter().map(|kind| kind.key()).collect();
    assert_eq!(keys.len(), HandOffKind::ALL.len());
}

#[test]
fn an_entry_reads_back_the_scene_and_seats_it_carries() {
    let entry = Entry::carrying(vec![
        CarryItem::Seat(SeatId::new(2)),
        CarryItem::Scene(SceneId::new(7)),
        CarryItem::Seat(SeatId::new(1)),
    ]);
    assert_eq!(entry.scene(), Some(SceneId::new(7)));
    assert_eq!(
        entry.seats().collect::<Vec<_>>(),
        [SeatId::new(2), SeatId::new(1)]
    );
    assert_eq!(Entry::empty().scene(), None);
    let hand_off = HandOff::new(
        MachineId::new("explore").expect("valid id"),
        entry.carry().to_vec(),
    )
    .expect("builds");
    assert_eq!(Entry::from_hand_off(&hand_off), entry);
}

#[test]
fn a_readout_cannot_be_blank() {
    assert_eq!(MachineReadout::new("").err(), Some(ReadoutError::Blank));
    assert_eq!(
        MachineReadout::new("  \n ").err(),
        Some(ReadoutError::Blank)
    );
    assert_eq!(
        MachineReadout::new("The party waits.")
            .expect("not blank")
            .as_str(),
        "The party waits."
    );
}

#[test]
fn every_machine_error_maps_to_one_closed_cause_and_a_record_failure_keeps_its_fault() {
    let id = || MachineId::new("sample").expect("valid id");
    let errors = [
        (
            MachineError::Load(LoadError::WrongMachine {
                expected: id(),
                found: id(),
            }),
            MachineFailureCause::SlotWrongMachine,
        ),
        (
            MachineError::Load(LoadError::UnsupportedVersion {
                found: 9,
                expected: 1,
            }),
            MachineFailureCause::SlotUnsupportedVersion,
        ),
        (
            MachineError::Load(LoadError::Malformed {
                version: 1,
                class: ErrorClass::Data,
            }),
            MachineFailureCause::SlotMalformed,
        ),
        (
            MachineError::Entry(crate::machine::EntryError::NoSeats),
            MachineFailureCause::EntryRefused,
        ),
        (
            MachineError::Slot(SlotError::NotEncodable {
                class: ErrorClass::Data,
            }),
            MachineFailureCause::StateInvalid,
        ),
        (
            MachineError::HandOff(HandOffError::TooManyCarryItems),
            MachineFailureCause::HandOffRefused,
        ),
        (
            MachineError::Projection(ReadoutError::Blank),
            MachineFailureCause::ProjectionBlank,
        ),
        (
            MachineError::Decline(DeclineError::EmptyReason),
            MachineFailureCause::DeclineInvalid,
        ),
        (
            MachineError::Result(crate::machine::ResultCause::FactsConflict),
            MachineFailureCause::ResultInvalid,
        ),
    ];
    let causes: BTreeSet<_> = errors.iter().map(|(_, cause)| cause.key()).collect();
    assert_eq!(causes.len(), MachineFailureCause::ALL.len());
    for (error, cause) in errors {
        assert_eq!(error.cause(), cause);
        let line = error.event(&scope(), correlation(3)).render_audit_line();
        assert!(
            line.starts_with("MACHINE failed table=1 scene=2 machine=sample"),
            "{line}"
        );
        assert!(line.ends_with(&format!("cause={}", cause.key())), "{line}");
        assert!(!error.to_string().is_empty());
    }
    for fault in LoadCause::ALL {
        let cause = MachineFailureCause::from(*fault);
        assert!(cause.key().starts_with("slot_"));
    }
}

#[test]
fn a_step_that_changes_the_state_name_is_a_transition_event_and_a_decline_is_a_decline_event() {
    let outcome = StepOutcome::Settled(Outcome::Applied);
    let moved = step_events(
        &scope(),
        correlation(1),
        &state("first"),
        &state("second"),
        &outcome,
    );
    assert_eq!(moved.len(), 1);
    assert!(
        moved[0]
            .render_audit_line()
            .ends_with("from=first to=second"),
        "{}",
        moved[0].render_audit_line()
    );
    let stayed = step_events(
        &scope(),
        correlation(1),
        &state("first"),
        &state("first"),
        &outcome,
    );
    assert!(stayed.is_empty());
    let declined = StepOutcome::Settled(Outcome::Declined(Decline::with_default_reason(
        DeclineCode::NotViable,
    )));
    let refused = step_events(
        &scope(),
        correlation(1),
        &state("first"),
        &state("first"),
        &declined,
    );
    assert_eq!(refused.len(), 1);
    assert!(refused[0].render_audit_line().ends_with("code=not_viable"));
    let waiting = StepOutcome::Waiting(crate::envelope::PendingId::new(4));
    let paused = step_events(
        &scope(),
        correlation(1),
        &state("first"),
        &state("second"),
        &waiting,
    );
    assert_eq!(paused.len(), 1, "waiting is a transition and not a decline");
}

#[test]
fn entering_and_exiting_have_their_own_event_builders_next_to_the_step_events() {
    let entered = entered_event(&scope(), correlation(1), state("first"));
    assert!(entered.render_audit_line().ends_with("state=first"));
    let carry = vec![
        CarryItem::Scene(SceneId::new(1)),
        CarryItem::Seat(SeatId::new(2)),
    ];
    let machine = HandOff::new(MachineId::new("lobby").expect("valid id"), carry).expect("builds");
    let exited = exited_event(&scope(), correlation(1), &machine);
    assert!(
        exited
            .render_audit_line()
            .ends_with("kind=machine target=lobby carried=2")
    );
    let release = HandOff::to(HandOffTarget::ReleaseLane, Vec::new()).expect("builds");
    let line = exited_event(&scope(), correlation(1), &release).render_audit_line();
    assert!(line.ends_with("kind=release_lane carried=0"), "{line}");
}

#[test]
fn a_step_result_that_changes_nothing_carries_no_state_and_reads_back_the_state_before() {
    let before = MachineSlot::encode(&spec(), &Sample { count: 1 }).expect("encodes");
    let after = MachineSlot::encode(&spec(), &Sample { count: 2 }).expect("encodes");
    let in_place = StepResult::applied_in_place(Vec::new());
    assert_eq!(in_place.state_after(&before), &before);
    let moved = StepResult::applied(after.clone(), Vec::new());
    assert_eq!(moved.state_after(&before), &after);
    let declined = StepResult::declined(Decline::with_default_reason(DeclineCode::NotViable));
    assert!(declined.state.is_none());
    assert!(declined.table_effects.is_empty() && declined.resource_requests.is_empty());
    assert_eq!(RequestKey::new(7).get(), 7);
}

fn widest_context() -> (TableScope, Correlation, MachineId) {
    let name = "m".repeat(crate::limits::MAXIMUM_NAME_LENGTH);
    let machine = MachineId::new(&name).expect("a name of the maximum length is valid");
    let scope = TableScope::of_table(TableId::new(u64::MAX))
        .in_scene(SceneId::new(u64::MAX))
        .in_machine(machine.clone());
    let correlation = Correlation {
        operation_id: OperationId::from_bytes(&[0xff; 16]).expect("valid id"),
        turn_seq: TurnSeq::new(u64::MAX),
        revision: SessionRevision::new(
            RecoveryEpoch::new(u64::MAX).expect("valid epoch"),
            u64::MAX,
        ),
    };
    (scope, correlation, machine)
}

#[test]
fn the_machine_effect_and_load_events_render_one_line_that_round_trips_through_json() {
    let (scope, correlation, machine) = widest_context();
    let longest = state(&"s".repeat(crate::limits::MAXIMUM_NAME_LENGTH));
    let events = [
        TableEvent::MachineEntered {
            scope: scope.clone(),
            correlation,
            state: longest.clone(),
        },
        TableEvent::MachineTransitioned {
            scope: scope.clone(),
            correlation,
            from: longest.clone(),
            to: longest,
        },
        TableEvent::MachineExited {
            scope: scope.clone(),
            correlation,
            kind: HandOffKind::ReturnToFrameBelow,
            target: Some(machine.clone()),
            carried: usize::MAX,
        },
        TableEvent::MachineFailed {
            scope: scope.clone(),
            correlation,
            cause: MachineFailureCause::SlotUnsupportedVersion,
        },
        TableEvent::EffectApplied {
            scope: scope.clone(),
            correlation,
            effect: TableEffectKind::HandOffRecorded,
        },
        TableEvent::EffectRefused {
            scope,
            correlation,
            effect: TableEffectKind::HandOffRecorded,
            cause: EffectCause::SceneNotActive,
        },
        TableEvent::LoadRefused {
            table: TableId::new(u64::MAX),
            record: RecordKind::MachineSlot,
            machine: Some(machine),
            version: u64::MAX,
            expected: u64::MAX,
            cause: LoadCause::UnsupportedVersion,
        },
    ];
    for event in events {
        let line = event.render_audit_line();
        assert!(!line.contains('\n'), "{line}");
        assert!(
            line.len() <= AUDIT_LINE_CAPACITY_BYTES,
            "{} bytes, over the reserved {AUDIT_LINE_CAPACITY_BYTES}: {line}",
            line.len()
        );
        assert_eq!(event.class(), EventClass::Always);
        let json = serde_json::to_string(&event).expect("serializes");
        let parsed: TableEvent = serde_json::from_str(&json).expect("parses");
        assert_eq!(parsed, event);
        assert_eq!(parsed.render_audit_line(), line);
    }
}

const CONTEXT: &str = "table=1 scene=2 machine=explore \
    operation_id=0000000000000000000000000000feed turn_seq=7 epoch=1 revision=9";

fn explore_scope() -> TableScope {
    TableScope::of_table(TableId::new(1))
        .in_scene(SceneId::new(2))
        .in_machine(MachineId::new("explore").expect("valid id"))
}

fn pinned_correlation() -> Correlation {
    Correlation {
        operation_id: OperationId::from_hex("0000000000000000000000000000feed").expect("valid id"),
        turn_seq: TurnSeq::new(7),
        revision: SessionRevision::new(RecoveryEpoch::FIRST, 9),
    }
}

/// The class A field names of every machine, effect and load event, in order (question 89). A
/// renamed or reordered key anywhere in a line fails here.
#[test]
fn the_class_a_lines_of_every_machine_effect_and_load_event_are_pinned_in_full() {
    let scope = explore_scope();
    let correlation = pinned_correlation();
    let lobby = MachineId::new("lobby").expect("valid id");
    let cases = [
        (
            TableEvent::MachineEntered {
                scope: scope.clone(),
                correlation,
                state: state("presenting"),
            },
            format!("MACHINE entered {CONTEXT} state=presenting"),
        ),
        (
            TableEvent::MachineTransitioned {
                scope: scope.clone(),
                correlation,
                from: state("presenting"),
                to: state("awaiting_roll"),
            },
            format!("MACHINE transitioned {CONTEXT} from=presenting to=awaiting_roll"),
        ),
        (
            TableEvent::MachineExited {
                scope: scope.clone(),
                correlation,
                kind: HandOffKind::Machine,
                target: Some(lobby.clone()),
                carried: 3,
            },
            format!("MACHINE exited {CONTEXT} kind=machine target=lobby carried=3"),
        ),
        (
            TableEvent::MachineExited {
                scope: scope.clone(),
                correlation,
                kind: HandOffKind::ReleaseLane,
                target: None,
                carried: 0,
            },
            format!("MACHINE exited {CONTEXT} kind=release_lane carried=0"),
        ),
        (
            TableEvent::MachineFailed {
                scope: scope.clone(),
                correlation,
                cause: MachineFailureCause::SlotMalformed,
            },
            format!("MACHINE failed {CONTEXT} cause=slot_malformed"),
        ),
        (
            TableEvent::EffectApplied {
                scope: scope.clone(),
                correlation,
                effect: TableEffectKind::SeatJoined,
            },
            format!("EFFECT applied {CONTEXT} effect=seat_joined"),
        ),
        (
            TableEvent::EffectRefused {
                scope,
                correlation,
                effect: TableEffectKind::SeatJoined,
                cause: EffectCause::TableFull,
            },
            format!("EFFECT refused {CONTEXT} effect=seat_joined cause=table_full"),
        ),
        (
            TableEvent::LoadRefused {
                table: TableId::new(1),
                record: RecordKind::MachineSlot,
                machine: Some(lobby),
                version: 9,
                expected: 1,
                cause: LoadCause::UnsupportedVersion,
            },
            "LOAD refused table=1 record=machine_slot machine=lobby version=9 expected=1 \
             cause=unsupported_version"
                .to_owned(),
        ),
        (
            TableEvent::LoadRefused {
                table: TableId::new(1),
                record: RecordKind::EffectIntent,
                machine: None,
                version: 2,
                expected: 1,
                cause: LoadCause::Malformed,
            },
            "LOAD refused table=1 record=effect_intent version=2 expected=1 cause=malformed"
                .to_owned(),
        ),
    ];
    for (event, expected) in cases {
        assert_eq!(event.render_audit_line(), expected);
    }
}

#[test]
fn the_machine_events_file_under_their_own_audit_areas() {
    let scope = explore_scope();
    let correlation = pinned_correlation();
    let entered = entered_event(&scope, correlation, state("gathering"));
    let applied = TableEvent::EffectApplied {
        scope,
        correlation,
        effect: TableEffectKind::SeatJoined,
    };
    assert_eq!(entered.area(), "MACHINE");
    assert_eq!(applied.area(), "EFFECT");
}
