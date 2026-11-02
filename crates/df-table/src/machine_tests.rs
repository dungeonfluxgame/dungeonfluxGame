use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::element::NameError;
use crate::event::{AUDIT_LINE_CAPACITY_BYTES, EventClass, TableEvent};
use crate::identity::{Correlation, MachineId, OperationId, SceneId, SeatId, TableId, TableScope};
use crate::identity::{RecoveryEpoch, SessionRevision, TurnSeq};
use crate::machine::{
    CarryItem, Entry, HandOff, HandOffError, MAXIMUM_CARRY_ITEMS, MachineError,
    MachineFailureCause, MachineReadout, MachineSpec, ReadoutError, Scope, SpecError, StateName,
    StepOutcome, SuspendPolicy, step_events,
};
use crate::machine_slot::{LoadError, LoadFault, MachineSlot, SlotError};
use crate::outcome::{Decline, DeclineCode, DeclineError, Outcome};
use crate::ports::PortName;
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
fn a_spec_refuses_a_duplicate_target_or_rules_action_and_dedupes_a_port() {
    let action = crate::ports::ActionId::new("investigate").expect("valid action");
    let with_target = spec().handing_off_to("lobby").expect("first target");
    assert_eq!(
        with_target.clone().handing_off_to("lobby").err(),
        Some(SpecError::DuplicateHandOffTarget(
            MachineId::new("lobby").expect("valid id")
        ))
    );
    assert!(matches!(
        with_target.clone().handing_off_to("Not Valid").err(),
        Some(SpecError::InvalidName(_))
    ));
    let with_action = spec()
        .with_rules_action(action.clone())
        .expect("first action");
    assert_eq!(
        with_action.with_rules_action(action.clone()).err(),
        Some(SpecError::DuplicateRulesAction(action))
    );
    let ported = spec()
        .requiring_port(PortName::Broker)
        .requiring_port(PortName::Broker);
    assert_eq!(ported.required_ports(), [PortName::Broker]);
}

#[test]
fn every_spec_error_reads_as_a_sentence() {
    let errors = [
        SpecError::InvalidName(NameError::Empty),
        SpecError::NoStates,
        SpecError::DuplicateState(state("a")),
        SpecError::ZeroSchemaVersion,
        SpecError::DuplicateHandOffTarget(MachineId::new("lobby").expect("valid id")),
        SpecError::DuplicateRulesAction(crate::ports::ActionId::new("look").expect("valid")),
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
fn scopes_and_suspend_policies_have_distinct_keys() {
    let scopes: BTreeSet<_> = Scope::ALL.iter().map(|scope| scope.key()).collect();
    assert_eq!(scopes.len(), Scope::ALL.len());
    let policies: BTreeSet<_> = SuspendPolicy::ALL
        .iter()
        .map(|policy| policy.key())
        .collect();
    assert_eq!(policies.len(), SuspendPolicy::ALL.len());
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Sample {
    count: u32,
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
        Err(LoadError::Malformed { version: 2 })
    );
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
fn every_load_fault_has_a_distinct_key_and_a_refusal_event_names_both_versions() {
    let keys: BTreeSet<_> = LoadFault::ALL.iter().map(|fault| fault.key()).collect();
    assert_eq!(keys.len(), LoadFault::ALL.len());
    let record = MachineSlot::from_parts(MachineId::new("sample").expect("valid id"), 9, json!({}));
    let error = LoadError::UnsupportedVersion {
        found: 9,
        expected: 2,
    };
    let event = error.refusal_event(&spec(), &record);
    assert_eq!(
        event.render_audit_line(),
        "LOAD refused machine=sample version=9 expected=2 cause=unsupported_version"
    );
    assert_eq!(event.class(), EventClass::Always);
    assert_eq!(error.fault(), LoadFault::UnsupportedVersion);
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
    assert_eq!(hand_off.target(), &target);
    assert_eq!(hand_off.carry(), carry);
    let too_many = vec![CarryItem::Seat(SeatId::new(1)); MAXIMUM_CARRY_ITEMS + 1];
    assert_eq!(
        HandOff::new(target, too_many).err(),
        Some(HandOffError::TooManyCarryItems)
    );
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
fn every_machine_error_maps_to_one_closed_cause_and_one_failure_event() {
    let errors = [
        (
            MachineError::Load(LoadError::Malformed { version: 1 }),
            MachineFailureCause::SlotRefused,
        ),
        (
            MachineError::Entry(crate::machine::EntryRefusal::NoSeats),
            MachineFailureCause::EntryRefused,
        ),
        (
            MachineError::Slot(SlotError::NotEncodable),
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
            target: machine.clone(),
            carried: usize::MAX,
        },
        TableEvent::MachineFailed {
            scope: scope.clone(),
            correlation,
            cause: MachineFailureCause::ProjectionBlank,
        },
        TableEvent::EffectApplied {
            scope: scope.clone(),
            correlation,
            effect: crate::effects::TableEffectKind::HandOffRecorded,
        },
        TableEvent::EffectRefused {
            scope,
            correlation,
            effect: crate::effects::TableEffectKind::HandOffRecorded,
            cause: crate::effects::EffectRefusal::SceneNotActive,
        },
        TableEvent::LoadRefused {
            machine,
            version: u64::MAX,
            expected: u64::MAX,
            cause: LoadFault::UnsupportedVersion,
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

#[test]
fn the_machine_events_file_under_their_own_audit_areas_in_the_pinned_field_order() {
    let (scope, correlation, _) = widest_context();
    let entered = TableEvent::MachineEntered {
        scope: scope.clone(),
        correlation,
        state: state("gathering"),
    };
    let applied = TableEvent::EffectApplied {
        scope,
        correlation,
        effect: crate::effects::TableEffectKind::SeatJoined,
    };
    assert_eq!(entered.area(), "MACHINE");
    assert_eq!(applied.area(), "EFFECT");
    let line = entered.render_audit_line();
    let keys: Vec<&str> = line
        .split(' ')
        .skip(2)
        .filter_map(|pair| pair.split_once('=').map(|(key, _)| key))
        .collect();
    assert_eq!(
        keys,
        [
            "table",
            "scene",
            "machine",
            "operation_id",
            "turn_seq",
            "epoch",
            "revision",
            "state"
        ]
    );
}

/// The machines are pure: they never name a port that reads time or draws dice, never read the
/// time they are given and never name another machine's module.
#[test]
fn the_machine_modules_name_no_clock_no_dice_no_broker_and_no_other_machine() {
    let sources = [
        ("lobby.rs", include_str!("lobby.rs")),
        ("explore.rs", include_str!("explore.rs")),
        ("machine.rs", include_str!("machine.rs")),
    ];
    let forbidden = [
        "Clock",
        "Broker",
        "Interpreter",
        "Narrator",
        "EventSink",
        "KnowledgeView",
        "ManualClock",
        "now_ms",
        "prepare",
        "legal_choices",
    ];
    for (name, source) in sources {
        let code: String = source
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n")
            .replace("MachineInput::Dice", "")
            .replace("PortName::RulesOwner", "");
        for token in code.split(|c: char| !(c.is_alphanumeric() || c == '_')) {
            let defines_the_input = name == "machine.rs";
            assert!(
                defines_the_input && token == "now_ms" || !forbidden.contains(&token),
                "{name} names {token}"
            );
            assert!(
                defines_the_input || !matches!(token, "Dice" | "RulesOwner"),
                "{name} names {token}"
            );
        }
    }
    let lobby = include_str!("lobby.rs");
    let explore = include_str!("explore.rs");
    assert!(
        !lobby.contains("crate::explore"),
        "the lobby imports explore"
    );
    assert!(
        !explore.contains("crate::lobby"),
        "explore imports the lobby"
    );
}
