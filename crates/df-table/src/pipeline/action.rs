//! Runs a plan, one action at a time, through CHECK, DECIDE and the machine's step, and commits
//! once (C.0, C.2, C.4, C.5).
//!
//! `run_plan` takes the seat's plan (a tap is a plan of one) and runs each action in order against
//! the table and the machine's state as the earlier steps left them: the rules owner is told what
//! the earlier steps already did (`PlanSoFar`), and the machine reads a copy of the table with
//! their staged effects applied, so the second Attack of a hero with one Action is declined
//! `economy_spent` and not spent twice. The longest valid prefix is committed together in one
//! APPLY (RES26 `PartiallyCompleted`: applied, plus the question about the first step that
//! failed). A step that opens a pending ends the plan: a plan outlives neither its turn nor its
//! bound (C.2), so the steps after it are dropped.
//!
//! `run_action` is the contract of RES27 for one action. The machine is asked whether it accepts
//! the intent and is always stepped, so a machine that does not accept it declines it in its own
//! words. An action the machine sends to the rules owner goes through CHECK and DECIDE first:
//! a decline ends the action; a harm that needs the seat's yes opens a `pending{choice}`; a ruling
//! is reused when the stored key and scope are equal, settled at once by a reviewed automatic
//! policy, or asked of the host as a `pending{ruling}` (`AwaitingAdjudicator`); a resolution that
//! awaits answers opens the pending it waits on. Every pending steps the machine with its id so
//! it reports `Waiting{pending_id}` (C.4 phase one; `hold.rs`). What the rules owner resolved (its
//! effects and typed facts) is committed with the machine's own effects (RES18).
//!
//! What a plan stages is kept only if the step that staged it ends well: a step that is declined or
//! asks a question takes its staged lines, and the ruling note it set, with it (`Turn::rewind`).
//!
//! It never reads player text and never draws a die except through DECIDE. Nothing changes until
//! `commit`: the staged changes are made together or not at all, and only in memory; the host's
//! commit is what makes them durable.

use std::borrow::Cow;

use crate::effects::{TableEffect, TableState};
use crate::envelope::PendingId;
use crate::event::{PipelineStep, TableEvent};
use crate::identity::{SceneId, SeatId};
use crate::machine::slot::MachineSlot;
use crate::machine::{
    Decision, Machine, MachineFailureCause, MachineInput, StepInput, StepOutcome, StepResult,
};
use crate::outcome::{Decline, DeclineCode, Outcome, OutcomeKind, PendingKind};
use crate::pending::{RulingSource, ScopedRuling};
use crate::ports::{ActionId, PlanSoFar, RulingAnswer, RulingKey, RulingScope};

use super::admit::{self, Reading};
use super::apply::{self, ClarificationChange, RulingChange, Staged, StagedRequest};
use super::check::{self, Checked, Settled};
use super::decide::{self, Decided};
use super::events::{ClarifyCause, PipelineEvent};
use super::hold::{self, RulingFlow};
use super::table_access::PipelineTable;
use super::turn::Turn;
use super::{Clarification, answer, narrate};

/// The reserved name of the intent to leave the party and go to another scene alone (C.6).
pub const SPLIT_ACTION: &str = "split";

/// What was settled before the first action of a plan runs.
#[derive(Debug, Clone, Default)]
pub(super) struct StepMode {
    /// The seat confirmed the first action already.
    pub(super) confirmed: bool,
    /// A ruling that answers the first action's open question.
    pub(super) ruling: Option<RulingAnswer>,
    /// A ruling to store with this commit (the host's answer).
    pub(super) store: Option<ScopedRuling>,
    /// A stored ruling whose reuse the host confirmed for this scene.
    pub(super) reconfirm: Option<(RulingKey, RulingScope, Option<SceneId>)>,
    /// The pending the machine waited on while the seat or the host answered: it is released
    /// before the action runs, so the action is stepped from the machine's normal state.
    pub(super) release_waiting: Option<PendingId>,
}

/// The machine's state and the staged changes while a plan runs. The state is borrowed from the
/// snapshot the turn started from until a step changes it.
pub(super) struct PlanState<'s> {
    pub(super) slot: Cow<'s, MachineSlot>,
    pub(super) staged: Staged,
}

/// Whether the machine wants to hand the table off, once the plan is applied. A machine that fails
/// to say is a machine failure recorded once with its cause, never a hand-off that is quietly not
/// asked for.
pub(super) fn wants_hand_off(
    turn: &Turn<'_, '_>,
    machine: &dyn Machine,
    slot: &MachineSlot,
) -> bool {
    match machine.wants_hand_off(slot) {
        Ok(wanted) => wanted,
        Err(error) => {
            turn.emit(&error.event(&turn.scope, turn.correlation));
            false
        }
    }
}

/// Why a plan stopped at an action: it was declined, it opened a pending, or it needs a pick.
pub(super) enum Stop {
    Declined(Decline),
    Pending(PendingKind),
    Clarify(Clarification),
}

/// How one action ended: applied, or the reason the plan stops here.
pub(super) type ActionEnd = Result<(), Stop>;

pub(super) fn stop_declined(decline: Decline) -> ActionEnd {
    Err(Stop::Declined(decline))
}

pub(super) fn stop_pending(kind: PendingKind) -> ActionEnd {
    Err(Stop::Pending(kind))
}

fn stop_clarify(clarification: Clarification) -> ActionEnd {
    Err(Stop::Clarify(clarification))
}

pub(super) fn run_plan(
    turn: &mut Turn<'_, '_>,
    table: &mut PipelineTable,
    seat: SeatId,
    plan: &[ActionId],
    reading: Reading,
    mode: StepMode,
    closing: Option<PendingId>,
) -> Outcome {
    let snapshot = table.facts().snapshot();
    let Some((machine, running)) = turn.pipeline.active_machine(table, &snapshot) else {
        return Outcome::Declined(Decline::with_default_reason(DeclineCode::NotViable));
    };
    let machine = machine.clone();
    if closing.is_none() && table.pending().open_in_scene(table.active_scene()) > 0 {
        // Something in this scene waits for an answer, and the machine waits on it: a second
        // pending cannot stack on the first (C.4, RES26). The way out is the answer or an exit.
        return Outcome::Declined(Decline::with_default_reason(DeclineCode::PendingOpen));
    }
    let mut state = PlanState {
        slot: Cow::Borrowed(running.slot),
        staged: Staged::new(),
    };
    if let Some(pending) = mode.release_waiting
        && let Some(released) = answer::release_machine(turn, table, &snapshot, pending)
    {
        state.staged.slot = Some(released.clone());
        state.slot = Cow::Owned(released);
    }
    if turn.wake_idle {
        state.staged.effects.push(TableEffect::TableResumed);
    }
    state.staged.clarification = ClarificationChange::Clear(seat);
    stage_ruling_changes(&mut state, &mode);
    let must_commit = closing.is_some() || mode.store.is_some() || mode.reconfirm.is_some();
    if let Some(id) = closing {
        state.staged.close.push(id);
    }
    let mut applied = 0;
    let mut stop = None;
    for (index, action) in plan.iter().enumerate() {
        let step_mode = if index == 0 {
            mode.clone()
        } else {
            StepMode::default()
        };
        let mark = turn.mark();
        let end = run_action(
            turn,
            table,
            machine.as_ref(),
            &mut state,
            seat,
            action,
            reading,
            &step_mode,
        );
        match end {
            Ok(()) => applied += 1,
            Err(reason) => {
                if !matches!(reason, Stop::Pending(_)) {
                    // A step that is declined or asks a question keeps nothing it staged.
                    turn.rewind(mark);
                }
                stop = Some(reason);
                break;
            }
        }
    }
    let opened = state.staged.open.len();
    if stop.is_some() && applied == 0 && opened == 0 && !must_commit {
        return match stop {
            Some(Stop::Declined(decline)) => Outcome::Declined(decline),
            Some(Stop::Clarify(clarification)) => {
                turn.clarification = Some(clarification);
                Outcome::NeedsClarification
            }
            _ => Outcome::Declined(Decline::with_default_reason(DeclineCode::NotViable)),
        };
    }
    let wants_hand_off = wants_hand_off(turn, machine.as_ref(), &state.slot);
    if let Err(decline) = apply::commit(turn, table, state.staged) {
        return Outcome::Declined(decline);
    }
    if let (true, Some(Stop::Declined(_))) = (applied > 0, &stop) {
        turn.stage_outcome(turn.pipeline_event(|scope, correlation| {
            PipelineEvent::PlanPartlyApplied {
                scope,
                correlation,
                applied,
                total: plan.len(),
            }
        }));
    }
    if applied > 0 {
        turn.hand_off_wanted = wants_hand_off;
        narrate::stage(turn, seat);
    }
    match stop {
        None => Outcome::Applied,
        Some(Stop::Pending(kind)) => Outcome::Pending(kind),
        Some(Stop::Declined(decline)) if applied == 0 => Outcome::Declined(decline),
        Some(Stop::Declined(decline)) => {
            turn.unfinished = Some(decline);
            Outcome::Applied
        }
        Some(Stop::Clarify(clarification)) => {
            turn.clarification = Some(clarification);
            Outcome::NeedsClarification
        }
    }
}

fn stage_ruling_changes(state: &mut PlanState<'_>, mode: &StepMode) {
    if let Some(ruling) = &mode.store {
        state
            .staged
            .rulings
            .push(RulingChange::Store(ruling.clone()));
    }
    if let Some((key, scope, scene)) = &mode.reconfirm {
        state.staged.rulings.push(RulingChange::Reconfirm {
            key: key.clone(),
            scope: *scope,
            scene: *scene,
        });
    }
}

/// The table a step reads: the table as it is, or, once an earlier step staged effects, a copy
/// with them applied, so the step sees what the plan already did (C.2).
fn table_for_step(
    turn: &Turn<'_, '_>,
    table: &PipelineTable,
    state: &PlanState<'_>,
) -> Result<Option<TableState>, Decline> {
    if state.staged.effects.is_empty() {
        return Ok(None);
    }
    let mut scratch = table.facts().clone();
    let scope = table.scope();
    match scratch.apply(
        &scope,
        table.state_correlation(turn.correlation),
        &state.staged.effects,
    ) {
        Ok(_) => Ok(Some(scratch)),
        Err(error) => {
            turn.emit(&error.event(&scope, turn.correlation));
            Err(Decline::with_default_reason(DeclineCode::NotViable))
        }
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "the stage context of one action: turn, table, machine, plan state, seat, action, reading and mode"
)]
fn run_action(
    turn: &mut Turn<'_, '_>,
    table: &PipelineTable,
    machine: &dyn Machine,
    state: &mut PlanState<'_>,
    seat: SeatId,
    action: &ActionId,
    reading: Reading,
    mode: &StepMode,
) -> ActionEnd {
    if action.as_str() == SPLIT_ACTION {
        return stop_declined(admit::split_decline(table, seat));
    }
    let scratch = match table_for_step(turn, table, state) {
        Ok(scratch) => scratch,
        Err(decline) => return stop_declined(decline),
    };
    let facts = scratch.as_ref().unwrap_or_else(|| table.facts());
    let input = MachineInput::Intent {
        seat,
        action: action.clone(),
    };
    let accepted = machine.accepts(facts, &input, &state.slot);
    let mut decision = None;
    let mut ruling_to_store = None;
    if accepted && machine.spec().rules_actions().contains(action) {
        let settled = Settled {
            harm_check: reading.needs_harm_check(),
            confirmed: mode.confirmed,
            ruled: mode.ruling.is_some(),
        };
        let started = turn.now();
        let so_far = PlanSoFar {
            steps: &turn.applied_steps,
            effects: &state.staged.effects,
        };
        let checked = check::check(turn, seat, action, settled, &so_far, started);
        let (prepared, ruling) = match checked {
            Checked::Declined(decline) => {
                turn.stage_done(PipelineStep::Check, started, Some(OutcomeKind::Declined));
                return stop_declined(decline);
            }
            Checked::Confirm(reason) => {
                turn.stage_done(PipelineStep::Check, started, Some(OutcomeKind::Pending));
                return hold::open_confirmation(
                    turn,
                    table,
                    (facts, machine),
                    state,
                    (seat, action),
                    reason,
                );
            }
            Checked::Ruling { request, draws } => {
                match hold::settle_ruling(
                    turn,
                    table,
                    (facts, machine),
                    state,
                    (seat, action),
                    &request,
                ) {
                    Ok(RulingFlow::Use { answer, store }) => {
                        turn.stage_done(PipelineStep::Check, started, None);
                        ruling_to_store = store;
                        (draws, Some(answer))
                    }
                    Ok(RulingFlow::Opened) => {
                        turn.stage_done(PipelineStep::Check, started, Some(OutcomeKind::Pending));
                        return stop_pending(PendingKind::Ruling);
                    }
                    Err(decline) => {
                        turn.stage_done(PipelineStep::Check, started, Some(OutcomeKind::Declined));
                        return stop_declined(decline);
                    }
                }
            }
            Checked::Ready(draws) => {
                turn.stage_done(PipelineStep::Check, started, None);
                (draws, mode.ruling.clone())
            }
        };
        let mut drawn = Vec::new();
        match decide::decide(turn, seat, action, &prepared, ruling.as_ref(), &mut drawn) {
            Decided::Applied(resolved) => {
                state
                    .staged
                    .effects
                    .extend(resolved.effects.iter().cloned());
                turn.result_facts.extend(resolved.facts.iter().cloned());
                decision = Some(Decision::Resolved(crate::ports::Resolution::Applied(
                    resolved,
                )));
            }
            Decided::Declined(decline) => return stop_declined(decline),
            Decided::Await(awaiting) => {
                return hold::await_answers(
                    turn,
                    table,
                    (facts, machine),
                    state,
                    (seat, action),
                    awaiting,
                    &drawn,
                );
            }
        }
    }
    let step = StepInput {
        correlation: turn.correlation,
        now_ms: turn.now(),
        table: facts,
        input,
        decision,
    };
    match machine.step(&state.slot, &step) {
        Ok(result) => {
            let end = settle_step(turn, machine, state, seat, action, result);
            if end.is_ok()
                && let Some(ruling) = ruling_to_store
            {
                stage_automatic_ruling(turn, state, ruling);
            }
            end
        }
        Err(error) => {
            turn.emit(&error.event(&turn.scope, turn.correlation));
            stop_declined(Decline::with_default_reason(DeclineCode::NotViable))
        }
    }
}

/// A reviewed automatic ruling is stored only once the action it settled was applied.
fn stage_automatic_ruling(
    turn: &mut Turn<'_, '_>,
    state: &mut PlanState<'_>,
    ruling: ScopedRuling,
) {
    let (key, tier) = (ruling.key().clone(), ruling.tier());
    turn.on_commit.push(
        turn.pipeline_event(|scope, correlation| PipelineEvent::RulingStored {
            scope,
            correlation,
            key,
            tier,
            source: RulingSource::ReviewedAutomatic,
        }),
    );
    state.staged.rulings.push(RulingChange::Store(ruling));
}

/// A machine broke the contract: recorded once, never guessed at.
pub(super) fn machine_broke_contract_decline(turn: &Turn<'_, '_>) -> Decline {
    turn.emit(&TableEvent::MachineFailed {
        scope: turn.scope.clone(),
        correlation: turn.correlation,
        cause: MachineFailureCause::StateInvalid,
    });
    Decline::with_default_reason(DeclineCode::NotViable)
}

fn machine_broke_contract(turn: &Turn<'_, '_>) -> ActionEnd {
    stop_declined(machine_broke_contract_decline(turn))
}

/// Takes a settled machine step into the staged changes. A machine reports a pending only as
/// `Waiting{pending_id}` for a pending the pipeline opened (O.7), so a settled `Pending` or a
/// `Waiting` nobody asked for is a machine failure.
fn settle_step(
    turn: &mut Turn<'_, '_>,
    machine: &dyn Machine,
    state: &mut PlanState<'_>,
    seat: SeatId,
    action: &ActionId,
    result: StepResult,
) -> ActionEnd {
    match &result.outcome {
        StepOutcome::Settled(Outcome::Declined(decline)) => {
            return stop_declined(decline.clone());
        }
        StepOutcome::Settled(Outcome::NeedsClarification) => {
            return match clarify_with_legal(turn, seat) {
                Ok(clarification) => stop_clarify(clarification),
                Err(decline) => stop_declined(decline),
            };
        }
        StepOutcome::Waiting(_) | StepOutcome::Settled(Outcome::Pending(_)) => {
            return machine_broke_contract(turn);
        }
        StepOutcome::Settled(Outcome::Applied) => {}
    }
    absorb(turn, machine, state, result);
    turn.acted = Some((seat, action.clone()));
    turn.applied_steps.push(action.clone());
    Ok(())
}

/// Stages what a step changed: the new state, the effects and the resource requests.
pub(super) fn absorb(
    turn: &mut Turn<'_, '_>,
    machine: &dyn Machine,
    state: &mut PlanState<'_>,
    result: StepResult,
) -> StepOutcome {
    let id = machine.spec().id().clone();
    if let Some(next) = result.state {
        // A state whose name cannot be read has no transition to record; `validate` and the
        // registry's load check are what refuse an unreadable state.
        if let (Ok(from), Ok(to)) = (machine.state_name(&state.slot), machine.state_name(&next))
            && from != to
        {
            turn.on_commit.push(TableEvent::MachineTransitioned {
                scope: turn.scope.clone(),
                correlation: turn.correlation,
                from,
                to,
            });
        }
        state.staged.slot = Some(next.clone());
        state.slot = Cow::Owned(next);
    }
    state.staged.effects.extend(result.table_effects);
    for request in result.resource_requests {
        state.staged.requests.push(StagedRequest {
            machine: id.clone(),
            key: request.key,
            purpose: request.purpose,
        });
    }
    result.outcome
}

fn clarify_with_legal(turn: &Turn<'_, '_>, seat: SeatId) -> Result<Clarification, Decline> {
    let started = turn.now();
    match turn
        .pipeline
        .ports
        .rules
        .legal_choices(turn.correlation, seat)
    {
        Ok(options) => Ok(Clarification {
            options,
            cause: ClarifyCause::Unclear,
            keeps_draft: false,
        }),
        Err(error) => {
            turn.boundary_failed(error.port, error.cause, started);
            Err(Decline::with_default_reason(DeclineCode::NotSupportedYet))
        }
    }
}
