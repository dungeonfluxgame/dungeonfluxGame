//! A deliberately broken machine for every conformance check (the C2 acceptance): each wrapper
//! below keeps one rule broken and the rest of a working machine, and the suite must name exactly
//! that check.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::Value;

use crate::conformance::{ConformanceCheck, ConformanceFixture, ConformanceReport, run};
use crate::effects::{TableEffect, TableState};
use crate::envelope::PendingId;
use crate::explore_tests::{entry_to_the_first_scene, explore_fixture};
use crate::identity::{MachineId, SeatId};
use crate::lobby_tests::{explore_for_neighbour, lobby, lobby_fixture};
use crate::machine::{
    Entry, HandOff, InspectReport, Machine, MachineError, MachineInput, MachineReadout,
    MachineSpec, ReadoutError, ResourceRequest, Scope, StateName, StepInput, StepOutcome,
    StepResult, SuspendPolicy,
};
use crate::machine_slot::{LoadError, MachineSlot};
use crate::outcome::{Decline, DeclineCode};
use crate::ports::{Audience, PurposeName};

/// One way to break a machine. Each variant breaks one rule and nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fault {
    /// The spec lists a hand-off target that is not registered.
    UnknownTarget,
    /// The spec leaves out a state the machine reaches.
    SpecOmitsAState,
    /// Two different states report the same name, so a transition cannot be seen.
    CollapsedStateNames,
    /// `step` turns a damaged record into a fresh decline instead of failing.
    StepSwallowsDamage,
    /// There is nothing to show once the game has started.
    NoProjectionOnceStarted,
    /// The inspect report names the wrong state.
    WrongInspectState,
    /// Loading a good record changes it.
    LoadChangesTheRecord,
    /// Loading accepts any record.
    LoadsAnything,
    /// `enter` accepts an entry the table disagrees with.
    EntryAcceptsAnything,
    /// `exit` names a machine the spec does not declare.
    ExitNamesUndeclaredTarget,
    /// `accepts` says yes to a timer nobody addressed to the machine.
    AcceptsTimers,
    /// `accepts` says yes to the dice of a pending that does not exist.
    AcceptsStrayDice,
    /// The machine reports waiting on a pending other than the one that was opened.
    WaitsOnTheWrongPending,
    /// `accepts` says yes to a resource result when no request is outstanding.
    AcceptsStrayResult,
    /// Every second call of `step` gives a different result.
    AlternatingResults,
    /// A step proposes a seat leaving that never sat.
    ProposesAnAbsentSeatLeaving,
    /// A step asks to hand off when the game has not started.
    AsksToLeaveEarly,
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
        ConformanceCheck::DiceAndWaiting => Fault::WaitsOnTheWrongPending,
        ConformanceCheck::ResourceResults => Fault::AcceptsStrayResult,
        ConformanceCheck::Determinism => Fault::AlternatingResults,
        ConformanceCheck::EffectsApply => Fault::ProposesAnAbsentSeatLeaving,
        ConformanceCheck::RunAlone => Fault::AsksToLeaveEarly,
    }
}

fn runs_over_explore(fault: Fault) -> bool {
    matches!(
        fault,
        Fault::EntryAcceptsAnything | Fault::WaitsOnTheWrongPending
    )
}

struct Faulty {
    inner: Arc<dyn Machine>,
    spec: MachineSpec,
    fault: Fault,
    valid_entry: Entry,
    intent_calls: AtomicU64,
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
        Fault::SpecOmitsAState => MachineSpec::new(
            "lobby",
            Scope::Table,
            1,
            &["gathering"],
            SuspendPolicy::FinishAndHold,
        )
        .expect("valid spec")
        .handing_off_to("explore")
        .expect("valid target"),
        _ => inner.spec().clone(),
    };
    Faulty {
        inner,
        spec,
        fault,
        valid_entry: entry_to_the_first_scene(),
        intent_calls: AtomicU64::new(0),
    }
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

    fn accepts(&self, input: &MachineInput, state: &MachineSlot) -> bool {
        let readable = self.inner.state_name(state).is_ok();
        match (self.fault, input) {
            (Fault::AcceptsTimers, MachineInput::Timer { .. })
            | (Fault::AcceptsStrayDice, MachineInput::Dice { .. })
            | (Fault::AcceptsStrayResult, MachineInput::ResourceResult { .. })
                if readable =>
            {
                true
            }
            _ => self.inner.accepts(input, state),
        }
    }

    fn step(&self, state: &MachineSlot, input: &StepInput) -> Result<StepResult, MachineError> {
        let mut result = self.inner.step(state, input);
        match self.fault {
            Fault::StepSwallowsDamage => {
                result = result.or_else(|_| {
                    Ok(StepResult::declined(
                        state,
                        Decline::with_default_reason(DeclineCode::NotViable),
                    ))
                });
            }
            Fault::AlternatingResults => {
                let is_intent = matches!(input.input, MachineInput::Intent { .. });
                let second_call =
                    is_intent && self.intent_calls.fetch_add(1, Ordering::Relaxed) % 2 == 1;
                if second_call && let Ok(result) = &mut result {
                    result.resource_requests.push(ResourceRequest {
                        purpose: PurposeName::new("plate").expect("valid purpose"),
                    });
                }
            }
            Fault::ProposesAnAbsentSeatLeaving if names_action(&input.input, "ready") => {
                if let Ok(result) = &mut result {
                    result.table_effects.push(TableEffect::SeatLeft {
                        seat: SeatId::new(99),
                    });
                }
            }
            Fault::AsksToLeaveEarly if names_action(&input.input, "ready") => {
                if let Ok(result) = &mut result {
                    result.asks_for_hand_off = true;
                }
            }
            Fault::WaitsOnTheWrongPending => {
                if let Ok(result) = &mut result
                    && let StepOutcome::Waiting(pending) = result.outcome
                {
                    result.outcome = StepOutcome::Waiting(PendingId::new(pending.get() + 1));
                }
            }
            Fault::AppliesWhatItDoesNotAccept => {
                if matches!(input.input, MachineInput::Timer { .. }) {
                    result = Ok(StepResult::applied(
                        state.clone(),
                        vec![TableEffect::SeatJoined {
                            seat: SeatId::new(50),
                        }],
                    ));
                }
            }
            _ => {}
        }
        result
    }

    fn exit(&self, state: &MachineSlot) -> Result<HandOff, MachineError> {
        let honest = self.inner.exit(state)?;
        match self.fault {
            Fault::ExitNamesUndeclaredTarget => Ok(HandOff::new(
                MachineId::new("nowhere").expect("valid id"),
                honest.carry().to_vec(),
            )?),
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

    fn project(
        &self,
        state: &MachineSlot,
        audience: Audience,
    ) -> Result<MachineReadout, MachineError> {
        let started = self.inner.state_name(state)?.as_str() == "started";
        if self.fault == Fault::NoProjectionOnceStarted && started {
            return Err(ReadoutError::Blank.into());
        }
        self.inner.project(state, audience)
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
            _ => self.inner.load(saved),
        }
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

#[test]
fn a_machine_broken_in_one_way_fails_that_check_and_only_that_check() {
    for check in ConformanceCheck::ALL {
        let report = run_broken(broken_for(*check));
        let expected: BTreeSet<ConformanceCheck> = [*check].into_iter().collect();
        assert_eq!(
            report.failed_checks(),
            expected,
            "{:?} should fail only {check:?}: {:#?}",
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

#[test]
fn a_machine_that_accepts_the_dice_of_no_pending_fails_the_dice_check() {
    let report = run_broken(Fault::AcceptsStrayDice);
    assert!(
        report
            .failed_checks()
            .contains(&ConformanceCheck::DiceAndWaiting)
    );
}

#[test]
fn a_machine_that_changes_the_table_on_input_it_does_not_accept_fails_the_accepts_check() {
    let report = run_broken(Fault::AppliesWhatItDoesNotAccept);
    assert!(report.failed_checks().contains(&ConformanceCheck::Accepts));
}

#[test]
fn a_finding_says_which_step_and_what_was_seen() {
    let report = run_broken(Fault::AsksToLeaveEarly);
    let finding = report.findings().first().expect("a finding");
    assert_eq!(finding.check, ConformanceCheck::RunAlone);
    assert!(finding.detail.starts_with("step 1:"), "{}", finding.detail);
    assert!(finding.detail.contains("asks_for_hand_off"));
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
