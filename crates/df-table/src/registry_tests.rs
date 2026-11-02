use std::collections::BTreeSet;

use crate::effects::TableState;
use crate::identity::MachineId;
use crate::lobby_tests::{action, explore_for_neighbour, lobby};
use crate::machine::{
    Entry, EntryRefusal, HandOff, InspectReport, Machine, MachineError, MachineInput,
    MachineReadout, MachineSpec, ReadoutError, Scope, StateName, StepInput, StepResult,
    SuspendPolicy,
};
use crate::machine_slot::{LoadError, MachineSlot};
use crate::ports::{Audience, PortName};
use crate::registry::{
    Enablement, EnablementReason, FlagState, MachineFlags, RegistryBuilder, RegistryError,
    RegistryErrorKind,
};

/// A machine that is only a spec: every call refuses, so a registry test sees nothing else.
struct SpecOnly {
    spec: MachineSpec,
}

fn spec_only(spec: MachineSpec) -> SpecOnly {
    SpecOnly { spec }
}

impl Machine for SpecOnly {
    fn spec(&self) -> &MachineSpec {
        &self.spec
    }

    fn enter(&self, _table: &TableState, _entry: &Entry) -> Result<MachineSlot, MachineError> {
        Err(EntryRefusal::NoSeats.into())
    }

    fn accepts(&self, _input: &MachineInput, _state: &MachineSlot) -> bool {
        false
    }

    fn step(&self, state: &MachineSlot, _input: &StepInput) -> Result<StepResult, MachineError> {
        Err(LoadError::Malformed {
            version: state.schema_version(),
        }
        .into())
    }

    fn exit(&self, _state: &MachineSlot) -> Result<HandOff, MachineError> {
        Err(EntryRefusal::NoSeats.into())
    }

    fn state_name(&self, _state: &MachineSlot) -> Result<StateName, MachineError> {
        Err(EntryRefusal::NoSeats.into())
    }

    fn project(
        &self,
        _state: &MachineSlot,
        _audience: Audience,
    ) -> Result<MachineReadout, MachineError> {
        Err(ReadoutError::Blank.into())
    }

    fn inspect(&self, _state: &MachineSlot) -> Result<InspectReport, MachineError> {
        Err(EntryRefusal::NoSeats.into())
    }
}

fn id(name: &str) -> MachineId {
    MachineId::new(name).expect("valid id")
}

fn seat_lane_spec(target: &str) -> MachineSpec {
    MachineSpec::new(
        "creation",
        Scope::Seat,
        1,
        &["choosing"],
        SuspendPolicy::FinishAndHold,
    )
    .expect("valid spec")
    .handing_off_to(target)
    .expect("valid target")
}

fn errors_of(builder: RegistryBuilder) -> Vec<RegistryError> {
    builder
        .build(PortName::ALL)
        .err()
        .expect("the registry is refused")
}

#[test]
fn the_two_real_machines_register_in_one_line_each_and_both_run() {
    let registry = RegistryBuilder::new()
        .register(lobby())
        .register(explore_for_neighbour())
        .build(PortName::ALL)
        .expect("valid registry");
    let report = registry.report();
    let enabled: Vec<&MachineId> = report.enabled().collect();
    assert_eq!(enabled, [&id("explore"), &id("lobby")]);
    for status in &report.machines {
        assert_eq!(status.enablement, Enablement::Enabled);
        assert_eq!(status.reason, EnablementReason::DefaultOn);
    }
    let scopes: Vec<Scope> = report.machines.iter().map(|status| status.scope).collect();
    assert_eq!(scopes, [Scope::Group, Scope::Table]);
    assert!(registry.machine(&id("lobby")).is_some());
    assert!(registry.machine(&id("ghost")).is_none());
}

#[test]
fn a_flag_turns_a_machine_off_or_on_and_the_report_says_why() {
    let flags = MachineFlags::default()
        .with_flag(id("lobby"), FlagState::On)
        .with_flag(id("explore"), FlagState::Off);
    assert_eq!(flags.flag(&id("explore")), Some(FlagState::Off));
    assert_eq!(flags.flag(&id("other")), None);
    let errors = errors_of(
        RegistryBuilder::new()
            .register(lobby())
            .register(explore_for_neighbour())
            .flags(flags),
    );
    assert_eq!(
        errors.iter().map(RegistryError::kind).collect::<Vec<_>>(),
        [RegistryErrorKind::HandOffTargetDisabled]
    );
    let flags = MachineFlags::default()
        .with_flag(id("lobby"), FlagState::On)
        .with_flag(id("explore"), FlagState::On)
        .with_flag(id("spare"), FlagState::Off);
    let spare = MachineSpec::new(
        "spare",
        Scope::Group,
        1,
        &["idle"],
        SuspendPolicy::FinishAndHold,
    )
    .expect("valid spec");
    let registry = RegistryBuilder::new()
        .register(lobby())
        .register(explore_for_neighbour())
        .register(spec_only(spare))
        .flags(flags)
        .build(PortName::ALL)
        .expect("valid registry");
    let report = registry.report();
    let by_id = |name: &str| {
        report
            .machines
            .iter()
            .find(|status| status.machine == id(name))
            .expect("reported")
    };
    assert_eq!(by_id("lobby").reason, EnablementReason::FlaggedOn);
    assert_eq!(by_id("spare").enablement, Enablement::Disabled);
    assert_eq!(by_id("spare").reason, EnablementReason::FlaggedOff);
    assert!(
        registry.machine(&id("spare")).is_none(),
        "a disabled machine is never returned"
    );
    assert_eq!(report.enabled().count(), 2);
}

/// Each scenario breaks exactly one rule; the map must reach every kind of error.
fn scenarios() -> Vec<(RegistryErrorKind, Vec<RegistryError>)> {
    let ghost = || id("ghost");
    let duplicate = errors_of(
        RegistryBuilder::new()
            .register(lobby())
            .register(lobby())
            .register(explore_for_neighbour()),
    );
    let unknown_flag = errors_of(
        RegistryBuilder::new()
            .register(lobby())
            .register(explore_for_neighbour())
            .flags(MachineFlags::default().with_flag(ghost(), FlagState::Off)),
    );
    let unknown_target = errors_of(RegistryBuilder::new().register(lobby()));
    let disabled_target = errors_of(
        RegistryBuilder::new()
            .register(lobby())
            .register(explore_for_neighbour())
            .flags(MachineFlags::default().with_flag(id("explore"), FlagState::Off)),
    );
    let scope = errors_of(
        RegistryBuilder::new()
            .register(explore_for_neighbour())
            .register(lobby())
            .register(spec_only(seat_lane_spec("explore"))),
    );
    let missing_port = RegistryBuilder::new()
        .register(lobby())
        .register(explore_for_neighbour())
        .build(&[PortName::Broker])
        .err()
        .expect("the registry is refused");
    let rules_without_port = errors_of(
        RegistryBuilder::new().register(spec_only(
            MachineSpec::new(
                "quiz",
                Scope::Group,
                1,
                &["asking"],
                SuspendPolicy::FinishAndHold,
            )
            .expect("valid spec")
            .with_rules_action(action("answer"))
            .expect("first action"),
        )),
    );
    let nothing_enabled = errors_of(
        RegistryBuilder::new()
            .register(lobby())
            .register(explore_for_neighbour())
            .flags(
                MachineFlags::default()
                    .with_flag(id("lobby"), FlagState::Off)
                    .with_flag(id("explore"), FlagState::Off),
            ),
    );
    vec![
        (RegistryErrorKind::DuplicateMachine, duplicate),
        (RegistryErrorKind::UnknownFlagTarget, unknown_flag),
        (RegistryErrorKind::UnknownHandOffTarget, unknown_target),
        (RegistryErrorKind::HandOffTargetDisabled, disabled_target),
        (RegistryErrorKind::ScopeRuleBroken, scope),
        (RegistryErrorKind::MissingPort, missing_port),
        (
            RegistryErrorKind::RulesActionsNeedRulesOwner,
            rules_without_port,
        ),
        (RegistryErrorKind::NoEnabledMachines, nothing_enabled),
    ]
}

#[test]
fn each_startup_error_is_reachable_and_each_scenario_breaks_exactly_one_rule() {
    let mut reached = BTreeSet::new();
    for (expected, errors) in scenarios() {
        let kinds: Vec<RegistryErrorKind> = errors.iter().map(RegistryError::kind).collect();
        assert_eq!(kinds, [expected], "{errors:?}");
        reached.insert(expected);
    }
    let all: BTreeSet<RegistryErrorKind> = RegistryErrorKind::ALL.iter().copied().collect();
    assert_eq!(reached, all, "an error kind has no scenario");
}

#[test]
fn startup_errors_name_the_machine_and_what_to_fix() {
    let by_kind: Vec<_> = scenarios();
    let find = |kind: RegistryErrorKind| {
        by_kind
            .iter()
            .find(|(candidate, _)| *candidate == kind)
            .map(|(_, errors)| errors[0].clone())
            .expect("a scenario")
    };
    assert_eq!(
        find(RegistryErrorKind::UnknownHandOffTarget),
        RegistryError::UnknownHandOffTarget {
            machine: id("lobby"),
            target: id("explore"),
        }
    );
    assert_eq!(
        find(RegistryErrorKind::ScopeRuleBroken),
        RegistryError::ScopeRuleBroken {
            machine: id("creation"),
            target: id("explore"),
            from: Scope::Seat,
            to: Scope::Group,
        }
    );
    assert_eq!(
        find(RegistryErrorKind::MissingPort),
        RegistryError::MissingPort {
            machine: id("explore"),
            port: PortName::RulesOwner,
        }
    );
    for (_, errors) in scenarios() {
        for error in errors {
            assert!(!error.to_string().is_empty());
        }
    }
    assert_eq!(find(RegistryErrorKind::NoEnabledMachines).machine(), None);
    assert_eq!(
        find(RegistryErrorKind::DuplicateMachine).machine(),
        Some(&id("lobby"))
    );
}

#[test]
fn every_problem_comes_back_at_once_in_a_fixed_order() {
    let errors = RegistryBuilder::new()
        .register(lobby())
        .register(spec_only(seat_lane_spec("ghost")))
        .flags(MachineFlags::default().with_flag(id("phantom"), FlagState::Off))
        .build(&[])
        .err()
        .expect("the registry is refused");
    let kinds: Vec<RegistryErrorKind> = errors.iter().map(RegistryError::kind).collect();
    assert_eq!(
        kinds,
        [
            RegistryErrorKind::UnknownFlagTarget,
            RegistryErrorKind::UnknownHandOffTarget,
            RegistryErrorKind::UnknownHandOffTarget,
        ]
    );
}
