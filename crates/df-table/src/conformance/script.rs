//! The stand-in for the pipeline that plays a machine's script (the real one is C3).
//!
//! For each scripted step it does what the pipeline will do: asks `accepts`, decides a rules
//! action with the scripted rules owner, opens a pending for a roll, draws the dice, calls
//! `step` (twice, to prove it deterministic), turns the step into events, commits the effects to
//! a table state, hands resource requests to the scripted broker and polls `wants_hand_off`. Then
//! it checks the result against what the script expects, and runs the per-state probes of
//! `probes` on every state it reaches. It owns no rule of its own: every finding names the check
//! of the contract that was broken. Pending handling here is a stand-in that C4 replaces.

use std::collections::VecDeque;
use std::sync::Arc;

use crate::effects::{ApplyOutcome, TableState};
use crate::envelope::PendingId;
use crate::event::TableEvent;
use crate::identity::{Correlation, SeatId, TableScope};
use crate::machine::slot::MachineSlot;
use crate::machine::{
    Decision, Machine, MachineInput, MachineSpec, StepInput, StepOutcome, StepResult,
    entered_event, step_events,
};
use crate::outcome::{Decline, Outcome};
use crate::ports::{
    ActionId, Broker, Clock, Dice, Draw, DrawRequest, Preparation, Resolution, RulesOwner,
};
use crate::testkit::{ManualClock, QueuedDice, ScriptedBroker, ScriptedRulesOwner, correlation};

use super::probes::{self, Context, PROBE_TABLE};
use super::{
    ConformanceCheck, ConformanceFixture, ConformanceReport, Expectation, ExpectedResult,
    ScriptedInput, ScriptedStep, guarded,
};

/// How far the clock moves before each step, so a machine that reads the time would show it.
const STEP_INTERVAL_MS: u64 = 100;

struct OpenRoll {
    pending: PendingId,
    seat: SeatId,
    action: ActionId,
    requests: Vec<DrawRequest>,
}

struct Driver {
    machine: Arc<dyn Machine>,
    neighbours: Vec<Arc<dyn Machine>>,
    spec: MachineSpec,
    actions: Vec<ActionId>,
    table: TableState,
    slot: MachineSlot,
    rules: ScriptedRulesOwner,
    dice: QueuedDice,
    broker: ScriptedBroker,
    clock: ManualClock,
    open_rolls: VecDeque<OpenRoll>,
    next_pending: u64,
    turn: u64,
}

/// What the pipeline's CHECK and DECIDE made of an intent before the machine saw it.
enum Decided {
    Decision(Decision),
    /// The rules owner declined the action, so the machine is not called.
    Declined(Decline),
    /// A port failed; the finding is already recorded.
    Failed,
}

pub(super) fn play(fixture: ConformanceFixture, report: &mut ConformanceReport) {
    let machine = fixture.machine;
    let spec = machine.spec().clone();
    let entered = guarded(report, "enter", |_| {
        (
            machine.enter(&fixture.table, &fixture.entry),
            machine.enter(&fixture.table, &fixture.entry),
        )
    });
    let Some((entered, again)) = entered else {
        return;
    };
    if entered != again {
        report.fail(
            ConformanceCheck::Determinism,
            "enter gave two different states",
        );
    }
    let slot = match entered {
        Ok(slot) => slot,
        Err(error) => {
            report.fail(
                ConformanceCheck::RunAlone,
                format!("the fixture's entry was refused: {error}"),
            );
            return;
        }
    };
    let mut actions: Vec<ActionId> = spec.rules_actions().to_vec();
    for step in &fixture.script {
        if let ScriptedInput::Player { action, .. } = &step.input
            && !actions.contains(action)
        {
            actions.push(action.clone());
        }
    }
    let mut driver = Driver {
        machine,
        neighbours: fixture.neighbours,
        spec,
        actions,
        table: fixture.table,
        slot,
        rules: fixture.rules,
        dice: fixture.dice,
        broker: fixture.broker,
        clock: ManualClock::default(),
        open_rolls: VecDeque::new(),
        next_pending: 1,
        turn: 0,
    };
    guarded(report, "after enter", |report| driver.start(report));
    for (index, step) in fixture.script.iter().enumerate() {
        guarded(report, &format!("step {index}"), |report| {
            driver.play_step(index, step, report);
        });
    }
    guarded(report, "load refusal", |report| {
        probes::load_refusal(driver.machine.as_ref(), &driver.slot, report);
    });
    guarded(report, "failure events", |report| {
        probes::failure_events(driver.machine.as_ref(), &driver.table, report);
    });
}

impl Driver {
    fn scope(&self) -> TableScope {
        let scope = TableScope::of_table(PROBE_TABLE);
        let scope = match self.table.active_scene() {
            Some(scene) => scope.in_scene(scene),
            None => scope,
        };
        scope.in_machine(self.spec.id().clone())
    }

    fn start(&self, report: &mut ConformanceReport) {
        let _ = self.check_slot("after enter", report);
        if let Ok(state) = self.machine.state_name(&self.slot) {
            report
                .events
                .push(entered_event(&self.scope(), correlation(0), state));
        }
    }

    fn check_slot(&self, label: &str, report: &mut ConformanceReport) -> Option<bool> {
        let context = Context {
            machine: self.machine.as_ref(),
            neighbours: &self.neighbours,
            actions: &self.actions,
        };
        probes::check_slot(&context, &self.table, &self.slot, label, report)
    }

    fn play_step(&mut self, index: usize, step: &ScriptedStep, report: &mut ConformanceReport) {
        self.turn += 1;
        self.clock.advance_ms(STEP_INTERVAL_MS);
        let probe = correlation(self.turn);
        let label = format!("step {index}");
        if let ScriptedInput::TableChange(effect) = &step.input {
            self.change_table(&label, probe, std::slice::from_ref(effect), report);
            return;
        }
        let Some((input, prepared)) = self.input_for(&step.input, probe, &label, report) else {
            return;
        };
        let accepted = self.machine.accepts(&self.table, &input, &self.slot);
        if accepted != step.expect.accepted {
            report.fail(
                ConformanceCheck::Accepts,
                format!(
                    "{label}: accepts said {accepted}, the script says {}",
                    step.expect.accepted
                ),
            );
        }
        let decision = match (prepared, &step.input, accepted) {
            (Some(decision), _, _) => Some(decision),
            (None, ScriptedInput::Player { seat, action }, true)
                if self.spec.rules_actions().contains(action) =>
            {
                match self.decide(probe, *seat, action, &label, report) {
                    Decided::Decision(decision) => Some(decision),
                    Decided::Declined(decline) => {
                        let outcome = StepOutcome::Settled(Outcome::Declined(decline));
                        self.record_declined_before_step(&outcome, probe, report);
                        check_result(&label, &step.expect, &outcome, report);
                        return;
                    }
                    Decided::Failed => return,
                }
            }
            (None, _, _) => None,
        };
        let step_input = StepInput {
            correlation: probe,
            now_ms: self.clock.now_ms(),
            table: &self.table,
            input,
            decision,
        };
        let first = self.machine.step(&self.slot, &step_input);
        if first != self.machine.step(&self.slot, &step_input) {
            report.fail(
                ConformanceCheck::Determinism,
                format!("{label}: the same state and input gave two different results"),
            );
        }
        match first {
            Ok(result) => self.commit(&label, step, probe, result, accepted, report),
            Err(error) => {
                report.events.push(error.event(&self.scope(), probe));
                report.fail(
                    ConformanceCheck::RunAlone,
                    format!("{label}: step failed: {error}"),
                );
            }
        }
    }

    /// The host changes the table between inputs; the machine is not called, but every probe runs
    /// again, so a machine that kept its own copy of a shared fact is caught following it.
    fn change_table(
        &mut self,
        label: &str,
        probe: Correlation,
        effects: &[crate::effects::TableEffect],
        report: &mut ConformanceReport,
    ) {
        let scope = self.scope();
        match self.table.apply(&scope, probe, effects) {
            Ok(ApplyOutcome::Applied(events)) => report.events.extend(events),
            Ok(ApplyOutcome::AlreadyApplied) => report.fail(
                ConformanceCheck::RunAlone,
                format!("{label}: the stand-in reused an operation id"),
            ),
            Err(error) => report.fail(
                ConformanceCheck::RunAlone,
                format!("{label}: the table change did not apply: {error}"),
            ),
        }
        let _ = self.check_slot(label, report);
    }

    /// The machine input for a scripted step, and the decision that goes with it when the step
    /// answers a roll. `None` when the script cannot be played.
    fn input_for(
        &mut self,
        scripted: &ScriptedInput,
        probe: Correlation,
        label: &str,
        report: &mut ConformanceReport,
    ) -> Option<(MachineInput, Option<Decision>)> {
        match scripted {
            ScriptedInput::Player { seat, action } => Some((
                MachineInput::Intent {
                    seat: *seat,
                    action: action.clone(),
                },
                None,
            )),
            ScriptedInput::Raw(input) => Some((input.clone(), None)),
            ScriptedInput::TableChange(_) => None,
            ScriptedInput::AnswerOpenRoll => {
                let Some(roll) = self.open_rolls.pop_front() else {
                    report.fail(
                        ConformanceCheck::RunAlone,
                        format!("{label}: the script answers a roll, but none is open"),
                    );
                    return None;
                };
                let draws = self.draw(probe, &roll.requests, label, report)?;
                let resolution =
                    self.resolve(probe, roll.seat, &roll.action, &draws, label, report)?;
                Some((
                    MachineInput::Dice {
                        pending: roll.pending,
                        draws,
                    },
                    Some(Decision::Resolved(resolution)),
                ))
            }
        }
    }

    fn draw(
        &self,
        probe: Correlation,
        requests: &[DrawRequest],
        label: &str,
        report: &mut ConformanceReport,
    ) -> Option<Vec<Draw>> {
        let mut draws = Vec::with_capacity(requests.len());
        for request in requests {
            match self.dice.draw(probe, *request) {
                Ok(draw) => draws.push(draw),
                Err(error) => {
                    report.fail(
                        ConformanceCheck::RunAlone,
                        format!("{label}: dice: {error}"),
                    );
                    return None;
                }
            }
        }
        Some(draws)
    }

    fn resolve(
        &self,
        probe: Correlation,
        seat: SeatId,
        action: &ActionId,
        draws: &[Draw],
        label: &str,
        report: &mut ConformanceReport,
    ) -> Option<Resolution> {
        match self.rules.resolve(probe, seat, action, draws) {
            Ok(resolution) => Some(resolution),
            Err(error) => {
                report.fail(
                    ConformanceCheck::RunAlone,
                    format!("{label}: rules owner: {error}"),
                );
                None
            }
        }
    }

    /// DECIDE for a rules action: prepare it, then resolve it at once or open a pending.
    fn decide(
        &mut self,
        probe: Correlation,
        seat: SeatId,
        action: &ActionId,
        label: &str,
        report: &mut ConformanceReport,
    ) -> Decided {
        let preparation = match self.rules.prepare(probe, seat, action) {
            Ok(preparation) => preparation,
            Err(error) => {
                report.fail(
                    ConformanceCheck::RunAlone,
                    format!("{label}: rules owner: {error}"),
                );
                return Decided::Failed;
            }
        };
        match preparation {
            Preparation::Declined(decline) => Decided::Declined(decline),
            Preparation::Ready { draws } if draws.is_empty() => {
                match self.resolve(probe, seat, action, &[], label, report) {
                    Some(resolution) => Decided::Decision(Decision::Resolved(resolution)),
                    None => Decided::Failed,
                }
            }
            Preparation::Ready { draws } => {
                let pending = PendingId::new(self.next_pending);
                self.next_pending += 1;
                self.open_rolls.push_back(OpenRoll {
                    pending,
                    seat,
                    action: action.clone(),
                    requests: draws,
                });
                Decided::Decision(Decision::RollPending(pending))
            }
        }
    }

    fn record_declined_before_step(
        &self,
        outcome: &StepOutcome,
        probe: Correlation,
        report: &mut ConformanceReport,
    ) {
        if let StepOutcome::Settled(Outcome::Declined(decline)) = outcome {
            report.events.push(TableEvent::InputDeclined {
                scope: self.scope(),
                correlation: probe,
                code: decline.code(),
            });
        }
    }

    fn commit(
        &mut self,
        label: &str,
        step: &ScriptedStep,
        probe: Correlation,
        result: StepResult,
        accepted: bool,
        report: &mut ConformanceReport,
    ) {
        self.check_transition(label, step, probe, &result, report);
        check_result(label, &step.expect, &result.outcome, report);
        self.check_promises(label, accepted, &result, report);
        self.commit_effects(label, probe, &result, report);
        for request in &result.resource_requests {
            if let Err(error) = self.broker.request(probe, &request.purpose) {
                report.fail(
                    ConformanceCheck::ResourceResults,
                    format!("{label}: the broker took no request: {error}"),
                );
            }
        }
        self.check_waiting(label, &result.outcome, report);
        if let Some(next) = result.state {
            self.slot = next;
        }
        if let Some(asks) = self.check_slot(label, report)
            && asks != step.expect.wants_hand_off
        {
            report.fail(
                ConformanceCheck::RunAlone,
                format!(
                    "{label}: wants_hand_off was {asks}, the script says {}",
                    step.expect.wants_hand_off
                ),
            );
        }
    }

    fn check_transition(
        &self,
        label: &str,
        step: &ScriptedStep,
        probe: Correlation,
        result: &StepResult,
        report: &mut ConformanceReport,
    ) {
        let before = self.machine.state_name(&self.slot);
        let after = self.machine.state_name(result.state_after(&self.slot));
        if let (Ok(before), Ok(after)) = (&before, &after) {
            let observed = (before.as_str(), after.as_str());
            let expected = step
                .expect
                .transition
                .unwrap_or((before.as_str(), before.as_str()));
            if observed != expected {
                report.fail(
                    ConformanceCheck::TransitionEvents,
                    format!("{label}: the state went {observed:?}, the script says {expected:?}"),
                );
            }
            report.events.extend(step_events(
                &self.scope(),
                probe,
                before,
                after,
                &result.outcome,
            ));
        }
    }

    /// What a result may change. An input the machine does not accept changes nothing. A decline
    /// or a clarification changes nothing. A waiting step may change only the state.
    fn check_promises(
        &self,
        label: &str,
        accepted: bool,
        result: &StepResult,
        report: &mut ConformanceReport,
    ) {
        let touches_table =
            !result.table_effects.is_empty() || !result.resource_requests.is_empty();
        let changes_state = result.state.is_some();
        if !accepted {
            if changes_state || touches_table {
                report.fail(
                    ConformanceCheck::Accepts,
                    format!("{label}: an input the machine does not accept changed something"),
                );
            }
            return;
        }
        let broken = match &result.outcome {
            StepOutcome::Settled(Outcome::Declined(_) | Outcome::NeedsClarification) => {
                changes_state || touches_table
            }
            StepOutcome::Waiting(_) | StepOutcome::Settled(Outcome::Pending(_)) => touches_table,
            StepOutcome::Settled(Outcome::Applied) => false,
        };
        if broken {
            report.fail(
                ConformanceCheck::DeclineChangesNothing,
                format!(
                    "{label}: a declined, clarified or waiting result changed more than it may"
                ),
            );
        }
    }

    fn commit_effects(
        &mut self,
        label: &str,
        probe: Correlation,
        result: &StepResult,
        report: &mut ConformanceReport,
    ) {
        let scope = self.scope();
        match self.table.apply(&scope, probe, &result.table_effects) {
            Ok(ApplyOutcome::Applied(events)) => report.events.extend(events),
            Ok(ApplyOutcome::AlreadyApplied) => report.fail(
                ConformanceCheck::RunAlone,
                format!("{label}: the stand-in reused an operation id"),
            ),
            Err(error) => {
                report.events.push(error.event(&scope, probe));
                report.fail(
                    ConformanceCheck::EffectsApply,
                    format!("{label}: an effect did not apply: {error}"),
                );
            }
        }
    }

    /// A pending the machine reports must be the one the pending handler opened last.
    fn check_waiting(&self, label: &str, outcome: &StepOutcome, report: &mut ConformanceReport) {
        let StepOutcome::Waiting(reported) = outcome else {
            return;
        };
        let opened = self.open_rolls.back().map(|roll| roll.pending);
        if opened != Some(*reported) {
            report.fail(
                ConformanceCheck::DiceAndWaiting,
                format!(
                    "{label}: the machine waits on {reported:?}, the pending opened was {opened:?}"
                ),
            );
        }
    }
}

/// Whether the outcome is the one the script expects. A disagreement over `Waiting` belongs to
/// the dice check; any other to the run.
fn check_result(
    label: &str,
    expect: &Expectation,
    outcome: &StepOutcome,
    report: &mut ConformanceReport,
) {
    let observed = match outcome {
        StepOutcome::Settled(settled) => ExpectedResult::Settled(settled.kind()),
        StepOutcome::Waiting(_) => ExpectedResult::Waiting,
    };
    if observed != expect.result {
        let waiting_involved =
            observed == ExpectedResult::Waiting || expect.result == ExpectedResult::Waiting;
        let check = if waiting_involved {
            ConformanceCheck::DiceAndWaiting
        } else {
            ConformanceCheck::RunAlone
        };
        report.fail(
            check,
            format!(
                "{label}: the step ended {observed:?}, the script says {:?}",
                expect.result
            ),
        );
    }
}
