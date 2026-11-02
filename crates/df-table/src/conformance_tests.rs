//! A deliberately broken machine for every conformance check (the C2 acceptance): each wrapper
//! below keeps one rule broken and the rest of a working machine, and the suite must name exactly
//! that check.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use serde_json::Value;

use crate::conformance::{ConformanceCheck, ConformanceFixture, ConformanceReport, run};
use crate::effects::{TableEffect, TableState};
use crate::envelope::PendingId;
use crate::explore_tests::{entry_to_the_first_scene, explore_fixture};
use crate::identity::{MachineId, SeatId};
use crate::lobby_tests::{FIRST_SCENE, explore_for_neighbour, lobby, lobby_fixture};
use crate::machine::{
    CarryItem, Entry, HandOff, InspectReport, Machine, MachineError, MachineInput, MachineReadout,
    MachineSpec, ReadoutError, RequestKey, ResourceRequest, Scope, StateName, StepInput,
    StepOutcome, StepResult, SuspendPolicy,
};
use crate::machine_slot::{LoadError, MachineSlot};
use crate::outcome::{Decline, DeclineCode, Outcome};
use crate::ports::{Audience, PortName, PurposeName};

/// One way to break a machine. Each variant breaks one rule and nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fault {
    /// The spec lists a hand-off target that is not registered.
    UnknownTarget,
    /// The spec requires a port the host does not wire.
    RequiresAnUnwiredPort,
    /// The spec leaves out a state the machine reaches.
    SpecOmitsAState,
    /// Two different states report the same name, so a transition cannot be seen.
    CollapsedStateNames,
    /// `step` turns a damaged record into a fresh decline instead of failing.
    StepSwallowsDamage,
    /// `step` panics on a damaged record.
    PanicsOnDamage,
    /// There is nothing to show once the game has started.
    NoProjectionOnceStarted,
    /// The readout differs between two calls.
    ProjectionChangesBetweenCalls,
    /// The inspect report names the wrong state.
    WrongInspectState,
    /// Loading a good record changes it.
    LoadChangesTheRecord,
    /// Loading accepts any record.
    LoadsAnything,
    /// Validation reads only the tag, so a record with a field gone still loads.
    ValidatesOnlyTheTag,
    /// `enter` accepts an entry the table disagrees with.
    EntryAcceptsAnything,
    /// `exit` names a machine the spec does not declare.
    ExitNamesUndeclaredTarget,
    /// `exit` carries a scene before the table has entered one.
    ExitCarriesASceneTheTableIsNotIn,
    /// `accepts` says yes to a timer nobody addressed to the machine.
    AcceptsTimers,
    /// `accepts` says yes to an engine proposal the spec never claimed.
    AcceptsProposals,
    /// `accepts` says yes to a seat that is not at the table.
    AcceptsSeatsNotAtTheTable,
    /// `accepts` says yes to the dice of a pending that does not exist.
    AcceptsStrayDice,
    /// The machine reports waiting on a pending other than the one that was opened.
    WaitsOnTheWrongPending,
    /// A waiting result also proposes a table effect.
    WaitingProposesAnEffect,
    /// `accepts` says yes to a resource result when no request used its key.
    AcceptsStrayResult,
    /// The ask for a hand-off is true once and never again.
    AskIsOneShot,
    /// The machine asks to hand off in states where it is not finished.
    AsksToLeaveEarly,
    /// Every second call of `step` gives a different result.
    AlternatingResults,
    /// A step proposes a seat leaving that never sat.
    ProposesAnAbsentSeatLeaving,
    /// A declined result still hands back a state.
    DeclineKeepsAState,
    /// An input that is not accepted still changes the table.
    AppliesWhatItDoesNotAccept,
}

/// The fault that breaks each check. The match has no wildcard, so a check without a broken
/// machine does not compile.
fn broken_for(check: ConformanceCheck) -> Fault {
    match check {
        ConformanceCheck::Registration => Fault::UnknownTarget,
        ConformanceCheck::StateNames => Fault::SpecOmitsAState,
        ConformanceCheck::TransitionEvents => Fault::CollapsedStateNames,
        ConformanceCheck::FailureEvents => Fault::StepSwallowsDamage,
        ConformanceCheck::Projection => Fault::NoProjectionOnceStarted,
        ConformanceCheck::InspectReport => Fault::WrongInspectState,
        ConformanceCheck::SavedState => Fault::LoadChangesTheRecord,
        ConformanceCheck::LoadRefusal => Fault::LoadsAnything,
        ConformanceCheck::EntryRefusal => Fault::EntryAcceptsAnything,
        ConformanceCheck::ExitHandOff => Fault::ExitNamesUndeclaredTarget,
        ConformanceCheck::Accepts => Fault::AcceptsTimers,
        ConformanceCheck::DeclineChangesNothing => Fault::DeclineKeepsAState,
        ConformanceCheck::DiceAndWaiting => Fault::WaitsOnTheWrongPending,
        ConformanceCheck::ResourceResults => Fault::AcceptsStrayResult,
        ConformanceCheck::HandOffAsk => Fault::AskIsOneShot,
        ConformanceCheck::Determinism => Fault::AlternatingResults,
        ConformanceCheck::EffectsApply => Fault::ProposesAnAbsentSeatLeaving,
        ConformanceCheck::NoPanic => Fault::PanicsOnDamage,
        ConformanceCheck::RunAlone => Fault::AsksToLeaveEarly,
    }
}

fn runs_over_explore(fault: Fault) -> bool {
    matches!(
        fault,
        Fault::EntryAcceptsAnything
            | Fault::WaitsOnTheWrongPending
            | Fault::WaitingProposesAnEffect
    )
}

struct Faulty {
    inner: Arc<dyn Machine>,
    spec: MachineSpec,
    fault: Fault,
    valid_entry: Entry,
    intent_calls: AtomicU64,
    readouts: AtomicU64,
    asked: AtomicBool,
}

fn faulty(fault: Fault) -> Faulty {
    let inner: Arc<dyn Machine> = if runs_over_explore(fault) {
        Arc::new(explore_for_neighbour())
    } else {
        Arc::new(lobby())
    };
    let spec = match fault {
        Fault::UnknownTarget => inner
            .spec()
            .clone()
            .handing_off_to("nowhere")
            .expect("a valid target name"),
        Fault::RequiresAnUnwiredPort => inner.spec().clone().requiring_port(PortName::Narrator),
        Fault::SpecOmitsAState => MachineSpec::new(
            "lobby",
            Scope::Table,
            1,
            &["gathering"],
            SuspendPolicy::FinishAndHold,
        )
        .expect("valid spec")
        .handing_off_to("explore")
        .expect("valid target")
        .with_unseated_action(crate::lobby_tests::action("join"))
        .expect("first unseated action"),
        _ => inner.spec().clone(),
    };
    Faulty {
        inner,
        spec,
        fault,
        valid_entry: entry_to_the_first_scene(),
        intent_calls: AtomicU64::new(0),
        readouts: AtomicU64::new(0),
        asked: AtomicBool::new(false),
    }
}

fn from_a_seat_at_the_table(input: &StepInput<'_>) -> bool {
    matches!(&input.input, MachineInput::Intent { seat, .. } if input.table.seats().contains(seat))
}

fn names_action(input: &MachineInput, wanted: &str) -> bool {
    matches!(input, MachineInput::Intent { action, .. } if action.as_str() == wanted)
}

impl Machine for Faulty {
    fn spec(&self) -> &MachineSpec {
        &self.spec
    }

    fn enter(&self, table: &TableState, entry: &Entry) -> Result<MachineSlot, MachineError> {
        let entered = self.inner.enter(table, entry);
        match self.fault {
            Fault::EntryAcceptsAnything => {
                entered.or_else(|_| self.inner.enter(table, &self.valid_entry))
            }
            _ => entered,
        }
    }

    fn accepts(&self, table: &TableState, input: &MachineInput, state: &MachineSlot) -> bool {
        let readable = self.inner.state_name(state).is_ok();
        match (self.fault, input) {
            (Fault::AcceptsTimers, MachineInput::Timer { .. })
            | (Fault::AcceptsProposals, MachineInput::Proposal { .. })
            | (Fault::AcceptsSeatsNotAtTheTable, MachineInput::Intent { .. })
            | (Fault::AcceptsStrayDice, MachineInput::Dice { .. })
            | (Fault::AcceptsStrayResult, MachineInput::ResourceResult { .. })
                if readable =>
            {
                true
            }
            _ => self.inner.accepts(table, input, state),
        }
    }

    fn step(&self, state: &MachineSlot, input: &StepInput<'_>) -> Result<StepResult, MachineError> {
        let mut result = self.inner.step(state, input);
        match self.fault {
            Fault::StepSwallowsDamage => {
                result = result.or_else(|_| {
                    Ok(StepResult::declined(Decline::with_default_reason(
                        DeclineCode::NotViable,
                    )))
                });
            }
            Fault::PanicsOnDamage if result.is_err() => panic!("a damaged record"),
            Fault::AlternatingResults => {
                let is_applied_intent = matches!(input.input, MachineInput::Intent { .. })
                    && matches!(&result, Ok(done) if done.outcome == StepOutcome::Settled(Outcome::Applied));
                let second_call =
                    is_applied_intent && self.intent_calls.fetch_add(1, Ordering::Relaxed) % 2 == 1;
                if second_call && let Ok(result) = &mut result {
                    result.resource_requests.push(ResourceRequest {
                        key: RequestKey::new(1),
                        purpose: PurposeName::new("plate").expect("valid purpose"),
                    });
                }
            }
            Fault::ProposesAnAbsentSeatLeaving
                if names_action(&input.input, "ready") && from_a_seat_at_the_table(input) =>
            {
                if let Ok(result) = &mut result {
                    result.table_effects.push(TableEffect::SeatLeft {
                        seat: SeatId::new(99),
                    });
                }
            }
            Fault::DeclineKeepsAState
                if names_action(&input.input, "start") && from_a_seat_at_the_table(input) =>
            {
                if let Ok(result) = &mut result
                    && matches!(result.outcome, StepOutcome::Settled(Outcome::Declined(_)))
                {
                    result.state = Some(state.clone());
                }
            }
            Fault::WaitsOnTheWrongPending => {
                if let Ok(result) = &mut result
                    && let StepOutcome::Waiting(pending) = result.outcome
                {
                    result.outcome = StepOutcome::Waiting(PendingId::new(pending.get() + 1));
                }
            }
            Fault::WaitingProposesAnEffect => {
                if let Ok(result) = &mut result
                    && matches!(result.outcome, StepOutcome::Waiting(_))
                {
                    result
                        .table_effects
                        .push(TableEffect::SceneEntered { scene: FIRST_SCENE });
                }
            }
            Fault::AppliesWhatItDoesNotAccept => {
                if matches!(input.input, MachineInput::Timer { .. }) {
                    result = Ok(StepResult::applied_in_place(vec![
                        TableEffect::SeatJoined {
                            seat: SeatId::new(50),
                        },
                    ]));
                }
            }
            _ => {}
        }
        result
    }

    fn wants_hand_off(&self, state: &MachineSlot) -> Result<bool, MachineError> {
        let honest = self.inner.wants_hand_off(state)?;
        match self.fault {
            Fault::AskIsOneShot => Ok(honest && !self.asked.swap(true, Ordering::Relaxed)),
            Fault::AsksToLeaveEarly => Ok(true),
            _ => Ok(honest),
        }
    }

    fn exit(&self, table: &TableState, state: &MachineSlot) -> Result<HandOff, MachineError> {
        let honest = self.inner.exit(table, state)?;
        match self.fault {
            Fault::ExitNamesUndeclaredTarget => Ok(HandOff::new(
                MachineId::new("nowhere").expect("valid id"),
                honest.carry().to_vec(),
            )?),
            Fault::ExitCarriesASceneTheTableIsNotIn => {
                let mut carry = honest.carry().to_vec();
                if !carry.contains(&CarryItem::Scene(FIRST_SCENE)) {
                    carry.push(CarryItem::Scene(FIRST_SCENE));
                }
                Ok(HandOff::to(honest.target().clone(), carry)?)
            }
            _ => Ok(honest),
        }
    }

    fn state_name(&self, state: &MachineSlot) -> Result<StateName, MachineError> {
        let honest = self.inner.state_name(state)?;
        match self.fault {
            Fault::CollapsedStateNames => Ok(StateName::new("gathering")?),
            _ => Ok(honest),
        }
    }

    fn validate(&self, state: &MachineSlot) -> Result<(), MachineError> {
        match self.fault {
            Fault::ValidatesOnlyTheTag => self.inner.state_name(state).map(|_| ()),
            _ => self.inner.validate(state),
        }
    }

    fn project(
        &self,
        table: &TableState,
        state: &MachineSlot,
        audience: Audience,
    ) -> Result<MachineReadout, MachineError> {
        let started = self.inner.state_name(state)?.as_str() == "started";
        if self.fault == Fault::NoProjectionOnceStarted && started {
            return Err(ReadoutError::Blank.into());
        }
        if self.fault == Fault::ProjectionChangesBetweenCalls {
            let call = self.readouts.fetch_add(1, Ordering::Relaxed);
            return Ok(MachineReadout::new(&format!("Reading number {call}."))?);
        }
        self.inner.project(table, state, audience)
    }

    fn inspect(&self, state: &MachineSlot) -> Result<InspectReport, MachineError> {
        let honest = self.state_name(state)?;
        let state_name = match self.fault {
            Fault::WrongInspectState => StateName::new("gathering")?,
            _ => honest,
        };
        Ok(InspectReport {
            machine: self.spec.id().clone(),
            state: state_name,
            schema_version: state.schema_version(),
        })
    }

    fn load(&self, saved: &MachineSlot) -> Result<MachineSlot, LoadError> {
        match self.fault {
            Fault::LoadsAnything => Ok(saved.clone()),
            Fault::LoadChangesTheRecord => {
                let loaded = self.inner.load(saved)?;
                Ok(MachineSlot::from_parts(
                    loaded.machine().clone(),
                    loaded.schema_version(),
                    Value::Null,
                ))
            }
            Fault::ValidatesOnlyTheTag => {
                if saved.machine() != self.spec.id() {
                    return Err(LoadError::WrongMachine {
                        expected: self.spec.id().clone(),
                        found: saved.machine().clone(),
                    });
                }
                if saved.schema_version() != self.spec.schema_version() {
                    return Err(LoadError::UnsupportedVersion {
                        found: saved.schema_version(),
                        expected: self.spec.schema_version(),
                    });
                }
                self.inner
                    .state_name(saved)
                    .map_err(|_| LoadError::Malformed {
                        version: saved.schema_version(),
                        class: crate::machine_slot::ErrorClass::Data,
                    })?;
                Ok(saved.clone())
            }
            _ => self.inner.load(saved),
        }
    }
}

fn wired_for(fault: Fault) -> Vec<PortName> {
    if runs_over_explore(fault) {
        vec![PortName::RulesOwner]
    } else {
        Vec::new()
    }
}

fn run_broken(fault: Fault) -> ConformanceReport {
    let base = if runs_over_explore(fault) {
        explore_fixture()
    } else {
        lobby_fixture()
    };
    run(ConformanceFixture {
        machine: Arc::new(faulty(fault)),
        wired: wired_for(fault),
        ..base
    })
}

#[test]
fn a_machine_with_no_fault_passes_the_same_fixtures() {
    for fixture in [lobby_fixture(), explore_fixture()] {
        let report = run(fixture);
        assert!(report.is_clean(), "{:#?}", report.findings());
    }
}

/// The checks a broken machine fails: its own, and nothing else, except where the break shows in
/// two places by its nature. An ask for a hand-off that vanishes after the first time it is read
/// is a `HandOffAsk` finding where it changes, and also makes the script's later step, which
/// expects the table to still be asked, end differently.
fn expected_failures(check: ConformanceCheck) -> BTreeSet<ConformanceCheck> {
    match check {
        ConformanceCheck::HandOffAsk => [ConformanceCheck::HandOffAsk, ConformanceCheck::RunAlone]
            .into_iter()
            .collect(),
        other => [other].into_iter().collect(),
    }
}

#[test]
fn a_machine_broken_in_one_way_fails_its_check_and_nothing_it_does_not_break() {
    for check in ConformanceCheck::ALL {
        let report = run_broken(broken_for(*check));
        let expected = expected_failures(*check);
        assert_eq!(
            report.failed_checks(),
            expected,
            "{:?} should fail exactly {expected:?}: {:#?}",
            broken_for(*check),
            report.findings()
        );
    }
}

#[test]
fn every_check_has_its_own_broken_machine() {
    let faults: BTreeSet<String> = ConformanceCheck::ALL
        .iter()
        .map(|check| format!("{:?}", broken_for(*check)))
        .collect();
    assert_eq!(faults.len(), ConformanceCheck::ALL.len());
}

/// The probes that do not depend on the fixture: each of these faults is caught although no
/// script line asks for it.
#[test]
fn faults_the_fixture_never_mentions_are_caught_by_the_probes_that_run_in_every_state() {
    let probed = [
        (Fault::AcceptsProposals, ConformanceCheck::Accepts),
        (Fault::AcceptsSeatsNotAtTheTable, ConformanceCheck::Accepts),
        (Fault::AcceptsStrayDice, ConformanceCheck::DiceAndWaiting),
        (
            Fault::ExitCarriesASceneTheTableIsNotIn,
            ConformanceCheck::ExitHandOff,
        ),
        (Fault::RequiresAnUnwiredPort, ConformanceCheck::Registration),
        (
            Fault::ProjectionChangesBetweenCalls,
            ConformanceCheck::Determinism,
        ),
        (Fault::ValidatesOnlyTheTag, ConformanceCheck::LoadRefusal),
        (
            Fault::WaitingProposesAnEffect,
            ConformanceCheck::DeclineChangesNothing,
        ),
        (Fault::AppliesWhatItDoesNotAccept, ConformanceCheck::Accepts),
        (Fault::AsksToLeaveEarly, ConformanceCheck::RunAlone),
    ];
    for (fault, check) in probed {
        let report = run_broken(fault);
        assert!(
            report.failed_checks().contains(&check),
            "{fault:?} should fail {check:?}: {:#?}",
            report.findings()
        );
    }
}

#[test]
fn a_panicking_machine_is_a_failed_check_and_the_run_goes_on() {
    let report = run_broken(Fault::PanicsOnDamage);
    let panics: Vec<_> = report
        .findings()
        .iter()
        .filter(|finding| finding.check == ConformanceCheck::NoPanic)
        .collect();
    assert_eq!(panics.len(), 1);
    assert!(panics[0].detail.contains("failure events"));
    assert!(
        report
            .events()
            .iter()
            .any(|event| event.render_audit_line().contains("MACHINE transitioned")),
        "the script ran before the panic was reached"
    );
}

#[test]
fn a_one_shot_ask_for_a_hand_off_is_named_where_it_changes() {
    let report = run_broken(Fault::AskIsOneShot);
    let finding = report
        .findings()
        .iter()
        .find(|finding| finding.check == ConformanceCheck::HandOffAsk)
        .expect("a finding");
    assert!(
        finding.detail.contains("strands the table"),
        "{}",
        finding.detail
    );
}

#[test]
fn a_finding_says_which_step_and_what_was_seen() {
    let report = run_broken(Fault::AsksToLeaveEarly);
    let finding = report.findings().first().expect("a finding");
    assert_eq!(finding.check, ConformanceCheck::RunAlone);
    assert!(finding.detail.starts_with("step 0:"), "{}", finding.detail);
    assert!(finding.detail.contains("wants_hand_off"));
}

#[test]
fn a_failed_run_still_records_every_event_it_reached() {
    let report = run_broken(Fault::ProposesAnAbsentSeatLeaving);
    let lines: Vec<String> = report
        .events()
        .iter()
        .map(|event| event.render_audit_line())
        .collect();
    assert!(
        lines
            .iter()
            .any(|line| line.contains("EFFECT refused") && line.contains("effect=seat_left")),
        "{lines:#?}"
    );
}

#[test]
fn a_script_that_cannot_be_played_is_reported_and_never_panics() {
    let mut fixture = explore_fixture();
    fixture.script = vec![crate::conformance::ScriptedStep::answering_open_roll(
        crate::conformance::Expectation::applied(),
    )];
    let report = run(fixture);
    assert!(report.failed_checks().contains(&ConformanceCheck::RunAlone));
    let mut refused = explore_fixture();
    refused.entry = Entry::empty();
    let report = run(refused);
    assert!(report.failed_checks().contains(&ConformanceCheck::RunAlone));
}

#[test]
fn a_table_change_that_does_not_apply_is_a_finding() {
    let mut fixture = lobby_fixture();
    fixture.script = vec![crate::conformance::ScriptedStep::table_change(
        TableEffect::SeatLeft {
            seat: SeatId::new(99),
        },
    )];
    let report = run(fixture);
    assert!(report.failed_checks().contains(&ConformanceCheck::RunAlone));
}
