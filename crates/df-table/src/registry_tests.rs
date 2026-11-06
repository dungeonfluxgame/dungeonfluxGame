use std::collections::BTreeSet;

use crate::effects::TableState;
use crate::identity::MachineId;
use crate::lobby_tests::{action, explore_for_neighbour, lobby};
use crate::machine::slot::{LoadError, MachineSlot};
use crate::machine::{
    CarryItem, Entry, EntryError, HandOff, HandOffTarget, InspectReport, Machine, MachineError,
    MachineInput, MachineReadout, MachineSpec, ReadoutError, Scope, StateName, StepInput,
    StepResult, SuspendPolicy,
};
use crate::ports::{Audience, PortName};
use crate::registry::{
    Enablement, EnablementReason, FlagState, HandOffRoute, HandOffTargetError, MachineFlags,
    RegistryBuilder, RegistryError, RegistryErrorKind, check_hand_off_route,
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
        Err(EntryError::NoSeats.into())
    }

    fn accepts(&self, _table: &TableState, _input: &MachineInput, _state: &MachineSlot) -> bool {
        false
    }

    fn step(
        &self,
        state: &MachineSlot,
        _input: &StepInput<'_>,
    ) -> Result<StepResult, MachineError> {
        Err(LoadError::UnsupportedVersion {
            found: state.schema_version(),
            expected: self.spec.schema_version(),
        }
        .into())
    }

    fn wants_hand_off(&self, _state: &MachineSlot) -> Result<bool, MachineError> {
        Err(EntryError::NoSeats.into())
    }

    fn exit(&self, _table: &TableState, _state: &MachineSlot) -> Result<HandOff, MachineError> {
        Err(EntryError::NoSeats.into())
    }

    fn state_name(&self, _state: &MachineSlot) -> Result<StateName, MachineError> {
        Err(EntryError::NoSeats.into())
    }

    fn validate(&self, _state: &MachineSlot) -> Result<(), MachineError> {
        Err(EntryError::NoSeats.into())
    }

    fn project(
        &self,
        _table: &TableState,
        _state: &MachineSlot,
        _audience: Audience,
    ) -> Result<MachineReadout, MachineError> {
        Err(ReadoutError::Blank.into())
    }

    fn inspect(&self, _state: &MachineSlot) -> Result<InspectReport, MachineError> {
        Err(EntryError::NoSeats.into())
    }
}

fn id(name: &str) -> MachineId {
    MachineId::new(name).expect("valid id")
}

fn stub_spec(name: &str, scope: Scope) -> MachineSpec {
    MachineSpec::new(name, scope, 1, &["idle"], SuspendPolicy::FinishAndHold).expect("valid spec")
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

fn table_stub() -> SpecOnly {
    spec_only(stub_spec("table_stub", Scope::Table))
}

fn group_stub() -> SpecOnly {
    spec_only(stub_spec("group_stub", Scope::Group))
}

fn errors_of(builder: RegistryBuilder) -> Vec<RegistryError> {
    builder
        .build(PortName::ALL)
        .err()
        .expect("the registry is refused")
}

fn kinds(errors: &[RegistryError]) -> Vec<RegistryErrorKind> {
    errors.iter().map(RegistryError::kind).collect()
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
            .with_flags(flags),
    );
    assert_eq!(
        kinds(&errors),
        [
            RegistryErrorKind::HandOffTargetDisabled,
            RegistryErrorKind::NotStartable
        ]
    );
    let flags = MachineFlags::default()
        .with_flag(id("lobby"), FlagState::On)
        .with_flag(id("explore"), FlagState::On)
        .with_flag(id("spare"), FlagState::Off);
    let registry = RegistryBuilder::new()
        .register(lobby())
        .register(explore_for_neighbour())
        .register(spec_only(stub_spec("spare", Scope::Group)))
        .with_flags(flags)
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
            .with_flags(MachineFlags::default().with_flag(id("ghost"), FlagState::Off)),
    );
    let unknown_target = errors_of(
        RegistryBuilder::new()
            .register(lobby())
            .register(group_stub()),
    );
    let disabled_target = errors_of(
        RegistryBuilder::new()
            .register(lobby())
            .register(explore_for_neighbour())
            .register(group_stub())
            .with_flags(MachineFlags::default().with_flag(id("explore"), FlagState::Off)),
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
        RegistryBuilder::new()
            .register(table_stub())
            .register(spec_only(
                stub_spec("quiz", Scope::Group)
                    .with_rules_action(action("answer"))
                    .expect("first action"),
            )),
    );
    let nothing_enabled = errors_of(
        RegistryBuilder::new()
            .register(lobby())
            .register(explore_for_neighbour())
            .with_flags(
                MachineFlags::default()
                    .with_flag(id("lobby"), FlagState::Off)
                    .with_flag(id("explore"), FlagState::Off),
            ),
    );
    let not_startable = errors_of(RegistryBuilder::new().register(table_stub()));
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
        (RegistryErrorKind::NotStartable, not_startable),
    ]
}

#[test]
fn each_startup_error_is_reachable_and_each_scenario_breaks_exactly_one_rule() {
    let mut reached = BTreeSet::new();
    for (expected, errors) in scenarios() {
        assert_eq!(kinds(&errors), [expected], "{errors:?}");
        reached.insert(expected);
    }
    let all: BTreeSet<RegistryErrorKind> = RegistryErrorKind::ALL.iter().copied().collect();
    assert_eq!(reached, all, "an error kind has no scenario");
}

#[test]
fn a_table_needs_a_machine_that_starts_it_and_a_machine_that_runs_a_scene() {
    let only_a_lane =
        errors_of(RegistryBuilder::new().register(spec_only(stub_spec("lane", Scope::Seat))));
    assert_eq!(
        only_a_lane,
        [
            RegistryError::NotStartable {
                missing: Scope::Table
            },
            RegistryError::NotStartable {
                missing: Scope::Group
            }
        ]
    );
    let no_table = errors_of(RegistryBuilder::new().register(group_stub()));
    assert_eq!(
        no_table,
        [RegistryError::NotStartable {
            missing: Scope::Table
        }]
    );
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
    assert_eq!(find(RegistryErrorKind::NotStartable).machine(), None);
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
        .with_flags(MachineFlags::default().with_flag(id("phantom"), FlagState::Off))
        .build(&[])
        .err()
        .expect("the registry is refused");
    assert_eq!(
        kinds(&errors),
        [
            RegistryErrorKind::UnknownFlagTarget,
            RegistryErrorKind::UnknownHandOffTarget,
            RegistryErrorKind::UnknownHandOffTarget,
            RegistryErrorKind::NotStartable,
        ]
    );
}

fn running_registry() -> crate::registry::Registry {
    RegistryBuilder::new()
        .register(lobby())
        .register(explore_for_neighbour())
        .register(spec_only(stub_spec("lane", Scope::Seat)))
        .build(PortName::ALL)
        .expect("valid registry")
}

fn to(name: &str) -> HandOff {
    HandOff::new(
        id(name),
        vec![CarryItem::Seat(crate::identity::SeatId::new(1))],
    )
    .expect("builds")
}

#[test]
fn the_hand_off_a_machine_returned_is_checked_against_the_declared_enabled_and_scope_rules() {
    let registry = running_registry();
    assert!(matches!(
        registry.check_hand_off(&id("lobby"), &to("explore")),
        Ok(HandOffRoute::To(machine)) if machine.spec().id() == &id("explore")
    ));
    assert!(matches!(
        registry.check_hand_off(&id("explore"), &to("lobby")),
        Ok(HandOffRoute::To(_))
    ));
    assert_eq!(
        registry.check_hand_off(&id("lobby"), &to("lane")).err(),
        Some(HandOffTargetError::UndeclaredTarget { target: id("lane") })
    );
    assert_eq!(
        registry.check_hand_off(&id("ghost"), &to("lobby")).err(),
        Some(HandOffTargetError::UnknownSource {
            machine: id("ghost")
        })
    );
}

#[test]
fn the_route_rules_refuse_an_unknown_a_disabled_and_a_forbidden_target() {
    let lobby = lobby();
    let from = lobby.spec();
    let hand_off = to("explore");
    let lookup = |scope: Option<(Scope, Enablement)>| move |_: &MachineId| scope;
    assert_eq!(
        check_hand_off_route(from, &hand_off, lookup(None)),
        Err(HandOffTargetError::UnknownTarget {
            target: id("explore")
        })
    );
    assert_eq!(
        check_hand_off_route(
            from,
            &hand_off,
            lookup(Some((Scope::Group, Enablement::Disabled)))
        ),
        Err(HandOffTargetError::TargetDisabled {
            target: id("explore")
        })
    );
    assert_eq!(
        check_hand_off_route(
            from,
            &hand_off,
            lookup(Some((Scope::Seat, Enablement::Enabled)))
        ),
        Err(HandOffTargetError::ScopeRuleBroken {
            from: Scope::Table,
            to: Scope::Seat
        })
    );
    assert_eq!(
        check_hand_off_route(
            from,
            &hand_off,
            lookup(Some((Scope::Group, Enablement::Enabled)))
        ),
        Ok(())
    );
}

#[test]
fn a_seat_lane_ends_by_releasing_itself_and_a_group_machine_may_return_below() {
    let registry = running_registry();
    let release = HandOff::to(HandOffTarget::ReleaseLane, Vec::new()).expect("builds");
    let below = HandOff::to(HandOffTarget::ReturnToFrameBelow, Vec::new()).expect("builds");
    assert!(matches!(
        registry.check_hand_off(&id("lane"), &release),
        Ok(HandOffRoute::ReleasesLane)
    ));
    assert!(matches!(
        registry.check_hand_off(&id("explore"), &below),
        Ok(HandOffRoute::ReturnsBelow)
    ));
    for (from, scope) in [("lobby", Scope::Table), ("explore", Scope::Group)] {
        assert_eq!(
            registry.check_hand_off(&id(from), &release).err(),
            Some(HandOffTargetError::LaneNotReleasable { scope }),
            "{from}"
        );
    }
    for (from, scope) in [("lobby", Scope::Table), ("lane", Scope::Seat)] {
        assert_eq!(
            registry.check_hand_off(&id(from), &below).err(),
            Some(HandOffTargetError::NothingBelow { scope }),
            "{from}"
        );
    }
    assert!(
        !HandOffTargetError::NothingBelow { scope: Scope::Seat }
            .to_string()
            .is_empty()
    );
}
