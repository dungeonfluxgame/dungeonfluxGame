//! Engine inputs: timers, resource results, proposals and controls (C.0, RES21, RES22).
//!
//! They skip ADMIT's seat checks, INTERPRET and DECIDE (no dice are needed) and go to the machine
//! or the table through APPLY like any turn, so every change of theirs is an input with an
//! operation id, a commit and a turn record (O.6).
//!
//! A timer named `PENDING_NUDGE_TIMER` finds the open pendings that have waited past their nudge
//! time and stages one mark and one `PENDING nudged` line for each; it resolves, spends and rolls
//! nothing (`window_timeout` none, RES26). Any other timer, a proposal and a resource result reach
//! the active machine only if it claims that kind of input (`InputClaim`) or, for a result, made
//! the request. A result is matched to the request that made it by its broker handle and judged
//! by the table against the stamp the request was made under (the scene and its generation, the
//! recovery epoch, the pause floor, RES42): a stale result, an unknown handle and a handle of
//! another machine are recorded once and dropped, and the request is forgotten. A
//! machine-requested hand-off is performed here: the machine must ask for it, no pending
//! resolution may be open in the scene (`pending_open`, RES26), and the registry checks the route
//! before the table moves; the new machine becomes the active one by an `ActiveMachineSet`
//! effect, so the table state and the pipeline never disagree about it. Stacks and seat lanes wait
//! for a machine that uses them.
//!
//! The controls are the changes the engine asks for that are not a story proposal: a seat's
//! presence (its held windows are held or released and nothing is spent, C.4), a ruling the host
//! revokes (RES48), a pending that ends without an answer (the exit of a window whose seat left
//! for good) and a table that plays again. An engine input is never declined to a seat: a refusal
//! is an `InputDeclined` event and the outcome of the input.

use std::borrow::Cow;

use crate::effects::TableEffect;
use crate::envelope::{EngineControl, EngineInput, ProposalOrigin};
use crate::event::{FailureCause, TableEvent};
use crate::machine::{
    Entry, MachineFailureCause, MachineInput, ResourceOutcome, Scope, StepInput, StepOutcome,
    entered_event, exited_event,
};
use crate::machine_spec::InputClaim;
use crate::outcome::{Decline, DeclineCode, Outcome};
use crate::pending::PendingMark;
use crate::ports::{PortName, Readiness};
use crate::registry::HandOffRoute;
use crate::table_state::{MachineHome, Presence};

use super::action::{self, PlanState};
use super::answer;
use super::apply::{self, RulingChange, Staged};
use super::events::PipelineEvent;
use super::table_access::PipelineTable;
use super::turn::Turn;

const MILLISECONDS_PER_SECOND: u64 = 1_000;

pub(super) fn run_engine(
    turn: &mut Turn<'_, '_>,
    table: &mut PipelineTable,
    input: &EngineInput,
) -> Outcome {
    match input {
        EngineInput::Control(control) => run_control(turn, table, control),
        EngineInput::Timer { timer } if timer.as_str() == super::PENDING_NUDGE_TIMER => {
            nudge_pendings(turn, table)
        }
        EngineInput::Timer { timer } => deliver(
            turn,
            table,
            InputClaim::Timer,
            MachineInput::Timer {
                timer: timer.clone(),
            },
        ),
        EngineInput::Proposal {
            origin: ProposalOrigin::Machine,
            ..
        } => hand_off(turn, table),
        EngineInput::Proposal { origin, proposer } => deliver(
            turn,
            table,
            InputClaim::EngineProposal,
            MachineInput::Proposal {
                origin: *origin,
                proposer: proposer.clone(),
            },
        ),
        EngineInput::ResourceResult { handle, readiness } => {
            resource_result(turn, table, handle.get(), *readiness)
        }
    }
}

fn not_viable() -> Outcome {
    Outcome::Declined(Decline::with_default_reason(DeclineCode::NotViable))
}

fn committed(turn: &mut Turn<'_, '_>, table: &mut PipelineTable, staged: Staged) -> Outcome {
    match apply::commit(turn, table, staged) {
        Ok(()) => Outcome::Applied,
        Err(decline) => Outcome::Declined(decline),
    }
}

fn nudge_pendings(turn: &mut Turn<'_, '_>, table: &mut PipelineTable) -> Outcome {
    let settings = turn.pipeline.settings;
    let ruling_ms = u64::from(settings.ruling_nudge_s()) * MILLISECONDS_PER_SECOND;
    let held_ms = u64::from(settings.held_nudge_s()) * MILLISECONDS_PER_SECOND;
    let mut staged = Staged::new();
    for nudge in table.pending().would_nudge(turn.now(), ruling_ms, held_ms) {
        staged.marks.push(PendingMark::Nudged(nudge.pending));
        turn.on_commit
            .push(
                turn.pipeline_event(|scope, correlation| PipelineEvent::PendingNudged {
                    scope,
                    correlation,
                    pending: nudge.pending,
                    kind: nudge.kind,
                    ms: nudge.waited_ms,
                }),
            );
    }
    committed(turn, table, staged)
}

fn run_control(
    turn: &mut Turn<'_, '_>,
    table: &mut PipelineTable,
    control: &EngineControl,
) -> Outcome {
    match control {
        EngineControl::SeatPresence { seat, presence } => {
            let mut staged = Staged::new();
            staged.effects.push(TableEffect::SeatPresenceChanged {
                seat: *seat,
                presence: *presence,
            });
            let seat = *seat;
            if *presence == Presence::Connected {
                for pending in table.pending().would_release(seat) {
                    turn.on_commit
                        .push(turn.pipeline_event(|scope, correlation| {
                            PipelineEvent::PendingReacquired {
                                scope,
                                correlation,
                                pending,
                                seat,
                            }
                        }));
                }
                staged.marks.push(PendingMark::Release(seat));
            } else {
                for pending in table.pending().would_hold(seat) {
                    turn.on_commit
                        .push(turn.pipeline_event(|scope, correlation| {
                            PipelineEvent::PendingHeld {
                                scope,
                                correlation,
                                pending,
                                seat,
                            }
                        }));
                }
                staged.marks.push(PendingMark::Hold(seat));
            }
            committed(turn, table, staged)
        }
        EngineControl::RevokeRuling { key, scope } => {
            let scene = table.active_scene();
            if table.rulings().find(key, *scope, scene).is_none() {
                return not_viable();
            }
            let mut staged = Staged::new();
            staged.rulings.push(RulingChange::Revoke {
                key: key.clone(),
                scope: *scope,
                scene,
            });
            let revoked = key.clone();
            turn.on_commit
                .push(
                    turn.pipeline_event(|scope, correlation| PipelineEvent::RulingRevoked {
                        scope,
                        correlation,
                        key: revoked,
                    }),
                );
            committed(turn, table, staged)
        }
        EngineControl::CancelPending { pending, cause } => {
            let Some(held) = table.pending().get(*pending).cloned() else {
                return Outcome::Declined(Decline::with_default_reason(DeclineCode::WindowClosed));
            };
            answer::cancel_pending(turn, table, &held, *cause)
        }
        EngineControl::ResumeTable => {
            let mut staged = Staged::new();
            staged.effects.push(TableEffect::TableResumed);
            committed(turn, table, staged)
        }
    }
}

/// A finished resource request: judged against the stamp it was made under, then given to the
/// machine that made it.
fn resource_result(
    turn: &mut Turn<'_, '_>,
    table: &mut PipelineTable,
    handle: u64,
    readiness: Readiness,
) -> Outcome {
    let outcome = match readiness {
        Readiness::Pending => return not_viable(),
        Readiness::Ready => ResourceOutcome::Ready,
        Readiness::Failed(cause) => ResourceOutcome::Failed(cause),
    };
    let started = turn.now();
    let Some(taken) = table.requests().peek_by_handle(handle) else {
        turn.boundary_failed(PortName::Broker, FailureCause::Stale, started);
        return not_viable();
    };
    let mut staged = Staged::new();
    staged.finished_requests.push(handle);
    let current = table.running_machine();
    if current.as_ref() != Some(&taken.machine) {
        turn.boundary_failed(PortName::Broker, FailureCause::Stale, started);
        return committed_decline(turn, table, staged);
    }
    if let Some(stamp) = taken.stamp
        && let Err(stale) = table.facts().snapshot().judge_result(&stamp)
    {
        turn.emit(&stale.event(&turn.scope, turn.correlation));
        return committed_decline(turn, table, staged);
    }
    if let ResourceOutcome::Failed(cause) = outcome {
        turn.boundary_failed(PortName::Broker, cause, started);
    }
    let input = MachineInput::ResourceResult {
        key: taken.key,
        outcome,
    };
    deliver_to_active(turn, table, input, staged)
}

/// Forgets a request whose result was refused, and declines the input.
fn committed_decline(
    turn: &mut Turn<'_, '_>,
    table: &mut PipelineTable,
    staged: Staged,
) -> Outcome {
    match apply::commit(turn, table, staged) {
        Ok(()) => not_viable(),
        Err(decline) => Outcome::Declined(decline),
    }
}

/// Gives an engine input to the active machine if it claims that kind of input.
fn deliver(
    turn: &mut Turn<'_, '_>,
    table: &mut PipelineTable,
    claim: InputClaim,
    input: MachineInput,
) -> Outcome {
    let claims = table
        .running_machine()
        .and_then(|id| turn.pipeline.registry.machine(&id))
        .is_some_and(|machine| machine.spec().claimed_inputs().contains(&claim));
    if !claims {
        return not_viable();
    }
    deliver_to_active(turn, table, input, Staged::new())
}

fn deliver_to_active(
    turn: &mut Turn<'_, '_>,
    table: &mut PipelineTable,
    input: MachineInput,
    staged: Staged,
) -> Outcome {
    let snapshot = table.facts().snapshot();
    let Some((machine, running)) = turn.pipeline.active_machine(table, &snapshot) else {
        return not_viable();
    };
    let machine = machine.clone();
    let mut state = PlanState {
        slot: Cow::Borrowed(running.slot),
        staged,
    };
    if !machine.accepts(table.facts(), &input, &state.slot) {
        return not_viable();
    }
    let step = StepInput {
        correlation: turn.correlation,
        now_ms: turn.now(),
        table: table.facts(),
        input,
        decision: None,
    };
    let result = match machine.step(&state.slot, &step) {
        Ok(result) => result,
        Err(error) => {
            turn.emit(&error.event(&turn.scope, turn.correlation));
            return not_viable();
        }
    };
    match &result.outcome {
        StepOutcome::Settled(Outcome::Applied) => {}
        StepOutcome::Settled(Outcome::Declined(decline)) => {
            return Outcome::Declined(decline.clone());
        }
        StepOutcome::Settled(_) | StepOutcome::Waiting(_) => return not_viable(),
    }
    action::absorb(turn, machine.as_ref(), &mut state, result);
    turn.hand_off_wanted = machine.wants_hand_off(&state.slot).unwrap_or(false);
    committed(turn, table, state.staged)
}

/// The active machine asked for a hand-off (RES21): the table moves if nothing is pending.
fn hand_off(turn: &mut Turn<'_, '_>, table: &mut PipelineTable) -> Outcome {
    let snapshot = table.facts().snapshot();
    let Some((machine, running)) = turn.pipeline.active_machine(table, &snapshot) else {
        return not_viable();
    };
    let machine = machine.clone();
    match machine.wants_hand_off(running.slot) {
        Ok(true) => {}
        Ok(false) => return not_viable(),
        Err(error) => return machine_failed(turn, &error),
    }
    if table.pending().open_in_scene(table.active_scene()) > 0 {
        return Outcome::Declined(Decline::with_default_reason(DeclineCode::PendingOpen));
    }
    let hand_off = match machine.exit(table.facts(), running.slot) {
        Ok(hand_off) => hand_off,
        Err(error) => return machine_failed(turn, &error),
    };
    let route = match turn
        .pipeline
        .registry
        .check_hand_off(machine.spec().id(), &hand_off)
    {
        Ok(route) => route,
        Err(_) => return refused_hand_off(turn),
    };
    let HandOffRoute::To(next) = route else {
        return Outcome::Declined(Decline::with_default_reason(DeclineCode::NotSupportedYet));
    };
    let Some(target) = hand_off.machine_target() else {
        return refused_hand_off(turn);
    };
    let entered = match next.enter(table.facts(), &Entry::from_hand_off(&hand_off)) {
        Ok(entered) => entered,
        Err(error) => return machine_failed(turn, &error),
    };
    let state_name = match next.state_name(&entered) {
        Ok(name) => name,
        Err(error) => return machine_failed(turn, &error),
    };
    let home = match (next.spec().scope(), snapshot.focus_scene()) {
        (Scope::Table, _) => MachineHome::Table,
        (Scope::Group | Scope::Seat, Some(scene)) => MachineHome::Scene { scene },
        (Scope::Group | Scope::Seat, None) => return not_viable(),
    };
    let mut staged = Staged::new();
    staged.effects.push(TableEffect::HandOffRecorded {
        from: machine.spec().id().clone(),
        to: target.clone(),
        carried: hand_off.carry().len(),
    });
    staged.effects.push(TableEffect::ActiveMachineSet {
        home,
        machine: target.clone(),
        slot: entered,
    });
    turn.on_commit
        .push(exited_event(&turn.scope, turn.correlation, &hand_off));
    let entered_scope = turn.scope.clone().in_machine(target.clone());
    turn.on_commit
        .push(entered_event(&entered_scope, turn.correlation, state_name));
    committed(turn, table, staged)
}

fn machine_failed(turn: &Turn<'_, '_>, error: &crate::machine::MachineError) -> Outcome {
    turn.emit(&error.event(&turn.scope, turn.correlation));
    not_viable()
}

fn refused_hand_off(turn: &Turn<'_, '_>) -> Outcome {
    turn.emit(&TableEvent::MachineFailed {
        scope: turn.scope.clone(),
        correlation: turn.correlation,
        cause: MachineFailureCause::HandOffRefused,
    });
    not_viable()
}
