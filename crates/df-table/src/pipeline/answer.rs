//! Answers by pending id: phase two of C.4, a confirmation, a ruling or a roll (RES26, K.4).
//!
//! An answer carries a pending id and goes to this handler, never to the machine's `accepts`. The
//! handler finds the pending (unknown or already resolved: declined `window_closed`), checks that
//! it still belongs to the table as it is now (the scene it was opened in is still the scene, and
//! the seat whose action it holds is still seated; otherwise it is cancelled, nothing spent, and
//! the answer is declined `window_closed`), takes the seat's answer into a copy of the window (a
//! seat with no offer: `not_your_turn`; a choice that was not offered or a second answer from the
//! same seat: `intent_invalid`), and when the window is complete finishes it by what the pending
//! holds:
//!
//! - a confirmation is the seat's own yes (the action then runs with the harm check settled) or
//!   its cancel (the uncommitted part is dropped and the input is applied, RES26);
//! - a ruling is the host's tap on the suggestion, an alternative or a refusal, or the
//!   requester's cancel; an answer is stored as a ruling keyed for reuse under the scene the
//!   question was asked in (RES48), a confirmed reuse is recorded for the scene, and the action
//!   runs with the ruling;
//! - a resolution that awaited answers resumes at the rules owner with every die drawn so far,
//!   re-validating against the table as it is now. Applied: the machine is stepped with the dice,
//!   what the rules owner resolved is committed with its effects, and the pending closes.
//!   Declined: the answer is not taken, the seat that gave it is asked again and the pending stays
//!   open. A further wait nests, up to `maximum_pending_nesting`; past it the answer is refused
//!   and the pending stays open.
//!
//! `cancel_pending` is the exit that needs no answer (C.4): a window whose seat left for good, or
//! one the host moves past, ends without spending, rolling or inventing anything. The rules owner
//! releases what it reserved (`abandon`) and a machine that waited on the pending is told
//! (`MachineInput::PendingCancelled`). A roll draws its dice only when the player answers, once:
//! the dice are kept in the pending even when the rules owner fails afterwards, so a later answer
//! resumes with them and never draws again. It never reads player text and never changes anything
//! but through `commit`.

use std::borrow::Cow;
use std::num::NonZeroU16;

use crate::envelope::{CancelCause, PendingId};
use crate::event::{FailureCause, TableEvent};
use crate::identity::SeatId;
use crate::machine::{Decision, MachineFailureCause, MachineInput, StepInput, StepOutcome};
use crate::outcome::{Decline, DeclineCode, Outcome, PendingKind};
use crate::pending::window::{AnswerRefusal, Completion, ResolutionWindow};
use crate::pending::{Continuation, PendingResolution, RulingSource, ScopedRuling};
use crate::ports::{
    ActionId, Awaiting, AwaitingKind, Draw, DrawRequest, Offer, PortName, Resolution, Resolved,
    RulingOption, RulingRequest, WindowAnswer,
};

use super::action::{self, PlanState, StepMode};
use super::admit::Reading;
use super::apply::{self, Staged};
use super::decide::{self, Entry};
use super::events::PipelineEvent;
use super::narrate;
use super::table_access::PipelineTable;
use super::turn::Turn;

pub(super) fn answer(
    turn: &mut Turn<'_, '_>,
    table: &mut PipelineTable,
    seat: SeatId,
    id: PendingId,
    choice: &ActionId,
) -> Outcome {
    let Some(pending) = table.pending().get(id).cloned() else {
        return declined(DeclineCode::WindowClosed);
    };
    if let Some(cause) = no_longer_current(table, &pending) {
        cancel_pending(turn, table, &pending, cause);
        return declined(DeclineCode::WindowClosed);
    }
    let window = match pending.window().answered(seat, choice) {
        Ok(window) => window,
        Err(AnswerRefusal::NotEligible) => return declined(DeclineCode::NotYourTurn),
        Err(AnswerRefusal::NotOffered | AnswerRefusal::AlreadyAnswered) => {
            return declined(DeclineCode::IntentInvalid);
        }
    };
    turn.on_commit.push(
        turn.pipeline_event(|scope, correlation| PipelineEvent::PendingAnswered {
            scope,
            correlation,
            pending: id,
            seat,
        }),
    );
    let updated = pending.with_window(window);
    if !updated.window().is_complete() {
        return keep_waiting(turn, table, updated);
    }
    match updated.continuation().clone() {
        Continuation::Confirm {
            seat: actor,
            action,
            ..
        } => {
            if *choice == turn.pipeline.choices.confirm {
                note_resolved(turn, &updated);
                let mode = StepMode {
                    confirmed: true,
                    ..StepMode::default()
                };
                action::run_plan(
                    turn,
                    table,
                    actor,
                    &[action],
                    Reading::Offered,
                    mode,
                    Some(id),
                )
            } else {
                cancel(turn, table, &updated)
            }
        }
        Continuation::Ruling {
            seat: actor,
            action,
            request,
            reuse,
        } => finish_ruling(
            turn,
            table,
            &updated,
            choice,
            (actor, action),
            request,
            reuse,
        ),
        Continuation::Resolve {
            seat: actor,
            action,
            nesting,
            roll,
        } => resume(turn, table, updated, seat, (actor, action), nesting, &roll),
    }
}

fn declined(code: DeclineCode) -> Outcome {
    Outcome::Declined(Decline::with_default_reason(code))
}

/// Why the pending no longer fits the table, if it does not: it was opened in another scene, or
/// the seat whose action it holds has left.
fn no_longer_current(table: &PipelineTable, pending: &PendingResolution) -> Option<CancelCause> {
    if pending.scene() != table.active_scene() {
        return Some(CancelCause::SceneChanged);
    }
    if !table.has_seat(pending.continuation().seat()) {
        return Some(CancelCause::SeatLeft);
    }
    None
}

fn note_resolved(turn: &mut Turn<'_, '_>, pending: &PendingResolution) {
    let (id, kind, ms) = (pending.id(), pending.kind(), pending.waited_ms(turn.now()));
    turn.on_commit.push(
        turn.pipeline_event(|scope, correlation| PipelineEvent::PendingResolved {
            scope,
            correlation,
            pending: id,
            kind,
            ms,
        }),
    );
}

/// Some seats have not answered yet: the answer is recorded and the pending stays open.
fn keep_waiting(
    turn: &mut Turn<'_, '_>,
    table: &mut PipelineTable,
    updated: PendingResolution,
) -> Outcome {
    let (id, kind) = (updated.id(), updated.kind());
    let mut staged = Staged::new();
    staged.replace.push(updated);
    match apply::commit(turn, table, staged) {
        Ok(()) => {
            turn.pending = Some(id);
            Outcome::Pending(kind)
        }
        Err(decline) => Outcome::Declined(decline),
    }
}

/// The seat or the requester dropped the uncommitted part: the input is applied with nothing
/// changed beyond closing the pending (RES26 `Cancelled`).
fn cancel(
    turn: &mut Turn<'_, '_>,
    table: &mut PipelineTable,
    pending: &PendingResolution,
) -> Outcome {
    note_resolved(turn, pending);
    let mut staged = Staged::new();
    staged.close.push(pending.id());
    match apply::commit(turn, table, staged) {
        Ok(()) => {
            turn.cancelled = true;
            Outcome::Applied
        }
        Err(decline) => Outcome::Declined(decline),
    }
}

/// Ends a pending without an answer and without spending anything (C.4): the rules owner releases
/// what it reserved, a machine that waited on the pending is told, and the pending closes.
pub(super) fn cancel_pending(
    turn: &mut Turn<'_, '_>,
    table: &mut PipelineTable,
    pending: &PendingResolution,
    cause: CancelCause,
) -> Outcome {
    let snapshot = table.facts().snapshot();
    let mut staged = Staged::new();
    staged.close.push(pending.id());
    let mut released = None;
    if let Continuation::Resolve {
        seat: actor,
        action,
        ..
    } = pending.continuation()
    {
        let started = turn.now();
        if let Err(error) = turn
            .pipeline
            .ports
            .rules
            .abandon(turn.correlation, *actor, action)
        {
            turn.boundary_failed(error.port, error.cause, started);
        }
        released = release_machine(turn, table, &snapshot, pending.id());
    }
    let (id, kind) = (pending.id(), pending.kind());
    turn.on_commit
        .push(
            turn.pipeline_event(|scope, correlation| PipelineEvent::PendingCancelled {
                scope,
                correlation,
                pending: id,
                kind,
                cause,
            }),
        );
    if let Some(slot) = released {
        staged.slot = Some(slot);
    }
    match apply::commit(turn, table, staged) {
        Ok(()) => {
            turn.cancelled = true;
            Outcome::Applied
        }
        Err(decline) => Outcome::Declined(decline),
    }
}

/// Tells the running machine its pending was cancelled, and returns the state it moved to.
fn release_machine(
    turn: &mut Turn<'_, '_>,
    table: &PipelineTable,
    snapshot: &crate::table_state::TableSnapshot,
    pending: PendingId,
) -> Option<crate::machine_slot::MachineSlot> {
    let (machine, running) = turn.pipeline.active_machine(table, snapshot)?;
    let input = MachineInput::PendingCancelled { pending };
    if !machine.accepts(table.facts(), &input, running.slot) {
        return None;
    }
    let step = StepInput {
        correlation: turn.correlation,
        now_ms: turn.now(),
        table: table.facts(),
        input,
        decision: None,
    };
    match machine.step(running.slot, &step) {
        Ok(result) => result.state,
        Err(error) => {
            turn.emit(&error.event(&turn.scope, turn.correlation));
            None
        }
    }
}

fn finish_ruling(
    turn: &mut Turn<'_, '_>,
    table: &mut PipelineTable,
    pending: &PendingResolution,
    choice: &ActionId,
    (actor, action): (SeatId, ActionId),
    request: RulingRequest,
    reuse: bool,
) -> Outcome {
    let choices = &turn.pipeline.choices;
    if *choice == choices.cancel_ruling {
        return cancel(turn, table, pending);
    }
    if *choice == choices.decline_ruling {
        note_resolved(turn, pending);
        let mut staged = Staged::new();
        staged.close.push(pending.id());
        return match apply::commit(turn, table, staged) {
            Ok(()) => declined(DeclineCode::NotViable),
            Err(decline) => Outcome::Declined(decline),
        };
    }
    let chosen = if *choice == choices.accept_suggestion {
        Some(request.suggested.clone())
    } else {
        request
            .alternatives
            .iter()
            .find(|option| option.answer.as_str() == choice.as_str())
            .cloned()
    };
    let Some(option) = chosen else {
        return declined(DeclineCode::IntentInvalid);
    };
    let asked_in = pending.scene();
    let same_as_stored = reuse && option.answer == request.suggested.answer;
    let mut mode = StepMode {
        confirmed: true,
        ruling: Some(option.answer.clone()),
        ..StepMode::default()
    };
    if same_as_stored {
        mode.reconfirm = Some((request.key.clone(), request.scope, asked_in));
    } else {
        mode.store = Some(ScopedRuling::new(
            request.key.clone(),
            request.scope,
            asked_in,
            option.answer.clone(),
            option.tier,
            RulingSource::Host,
            action.clone(),
        ));
        stored_event(turn, &request, &option);
    }
    note_resolved(turn, pending);
    action::run_plan(
        turn,
        table,
        actor,
        &[action],
        Reading::Offered,
        mode,
        Some(pending.id()),
    )
}

fn stored_event(turn: &mut Turn<'_, '_>, request: &RulingRequest, option: &RulingOption) {
    let (key, tier) = (request.key.clone(), option.tier);
    turn.on_commit.push(
        turn.pipeline_event(|scope, correlation| PipelineEvent::RulingStored {
            scope,
            correlation,
            key,
            tier,
            source: RulingSource::Host,
        }),
    );
}

/// Phase two of a resolution that awaited answers.
fn resume(
    turn: &mut Turn<'_, '_>,
    table: &mut PipelineTable,
    updated: PendingResolution,
    answering: SeatId,
    (actor, action): (SeatId, ActionId),
    nesting: u32,
    roll: &[u16],
) -> Outcome {
    let snapshot = table.facts().snapshot();
    let Some((machine, running)) = turn.pipeline.active_machine(table, &snapshot) else {
        return declined(DeclineCode::NotViable);
    };
    let machine = machine.clone();
    let mut state = PlanState {
        slot: Cow::Borrowed(running.slot),
        staged: Staged::new(),
    };
    let mut drawn = match updated.restored_draws() {
        Ok(drawn) => drawn,
        Err(_) => {
            turn.boundary_failed(PortName::Dice, FailureCause::InvalidAnswer, turn.started_ms);
            return declined(DeclineCode::NotViable);
        }
    };
    let requests: Vec<DrawRequest> = roll
        .iter()
        .filter_map(|sides| NonZeroU16::new(*sides))
        .map(|sides| DrawRequest { sides })
        .collect();
    if let Err(decline) = decide::draw_all(turn, &requests, &mut drawn) {
        return Outcome::Declined(decline);
    }
    let answers: Vec<WindowAnswer> = updated
        .window()
        .answers()
        .map(|(seat, choice)| WindowAnswer {
            seat,
            choice: choice.clone(),
        })
        .collect();
    let entry = Entry::Resume {
        answers: &answers,
        ruling: None,
    };
    match decide::resolve_loop(turn, actor, &action, &mut drawn, entry) {
        Ok(Resolution::Applied(resolved)) => {
            let who = (actor, answering);
            finish_resolution(
                turn, table, &machine, &mut state, &updated, who, &action, resolved, &drawn,
            )
        }
        Ok(Resolution::Await(next)) => nest(
            turn,
            table,
            updated,
            answering,
            (actor, action),
            nesting,
            next,
        ),
        Ok(Resolution::Declined(decline)) | Err(decline) => {
            ask_again(turn, table, &updated, answering, decline)
        }
        Ok(Resolution::NeedsDraws(_)) => ask_again(
            turn,
            table,
            &updated,
            answering,
            Decline::with_default_reason(DeclineCode::NotSupportedYet),
        ),
    }
}

/// The rules owner settled it: the machine is stepped with the dice, what the rules owner resolved
/// is committed with the machine's effects, and the pending closes.
#[allow(
    clippy::too_many_arguments,
    reason = "the turn, the table, the machine, its state, the pending, who acts and answers, the action and what was resolved"
)]
fn finish_resolution(
    turn: &mut Turn<'_, '_>,
    table: &mut PipelineTable,
    machine: &std::sync::Arc<dyn crate::machine::Machine>,
    state: &mut PlanState<'_>,
    updated: &PendingResolution,
    (actor, answering): (SeatId, SeatId),
    action: &ActionId,
    resolved: Resolved,
    drawn: &[Draw],
) -> Outcome {
    let step = StepInput {
        correlation: turn.correlation,
        now_ms: turn.now(),
        table: table.facts(),
        input: MachineInput::Dice {
            pending: updated.id(),
            draws: drawn.to_vec(),
        },
        decision: Some(Decision::Resolved(Resolution::Applied(resolved.clone()))),
    };
    let result = match machine.step(&state.slot, &step) {
        Ok(result) => result,
        Err(error) => {
            turn.emit(&error.event(&turn.scope, turn.correlation));
            return declined(DeclineCode::NotViable);
        }
    };
    match &result.outcome {
        StepOutcome::Settled(Outcome::Applied) => {}
        StepOutcome::Settled(Outcome::Declined(decline)) => {
            let decline = decline.clone();
            return ask_again(turn, table, updated, answering, decline);
        }
        StepOutcome::Waiting(_) | StepOutcome::Settled(_) => {
            turn.emit(&TableEvent::MachineFailed {
                scope: turn.scope.clone(),
                correlation: turn.correlation,
                cause: MachineFailureCause::StateInvalid,
            });
            return declined(DeclineCode::NotViable);
        }
    }
    state.staged.effects.extend(resolved.effects);
    turn.result_facts.extend(resolved.facts);
    action::absorb(turn, machine.as_ref(), state, result);
    note_resolved(turn, updated);
    state.staged.close.push(updated.id());
    let wants_hand_off = machine.wants_hand_off(&state.slot);
    let staged = std::mem::replace(&mut state.staged, Staged::new());
    if let Err(decline) = apply::commit(turn, table, staged) {
        return Outcome::Declined(decline);
    }
    turn.acted = Some((actor, action.clone()));
    turn.applied_steps.push(action.clone());
    turn.hand_off_wanted = wants_hand_off.unwrap_or(false);
    narrate::stage(turn, actor);
    Outcome::Applied
}

/// The resolution waits on a further stage: the pending takes the new offers and keeps its dice.
fn nest(
    turn: &mut Turn<'_, '_>,
    table: &mut PipelineTable,
    updated: PendingResolution,
    answering: SeatId,
    (actor, action): (SeatId, ActionId),
    nesting: u32,
    next: Awaiting,
) -> Outcome {
    let too_deep = nesting >= turn.pipeline.limits.maximum_pending_nesting;
    let built = match next.kind {
        AwaitingKind::Roll => {
            let offer = Offer {
                seat: actor,
                choices: vec![turn.pipeline.choices.roll.clone()],
            };
            let sides = next
                .draws
                .iter()
                .map(|request| request.sides.get())
                .collect();
            ResolutionWindow::new(vec![offer], Completion::AnyOffer)
                .map(|window| (PendingKind::Roll, window, sides))
        }
        AwaitingKind::Choice => ResolutionWindow::new(next.offers, Completion::EveryOffer)
            .map(|window| (PendingKind::Choice, window, Vec::new())),
        AwaitingKind::Reaction => ResolutionWindow::new(next.offers, Completion::EveryOffer)
            .map(|window| (PendingKind::Reaction, window, Vec::new())),
    };
    let Ok((kind, window, roll)) = built else {
        turn.boundary_failed(
            PortName::RulesOwner,
            FailureCause::InvalidAnswer,
            turn.started_ms,
        );
        let decline = Decline::with_default_reason(DeclineCode::NotSupportedYet);
        return ask_again(turn, table, &updated, answering, decline);
    };
    if too_deep {
        turn.boundary_failed(
            PortName::RulesOwner,
            FailureCause::InvalidAnswer,
            turn.started_ms,
        );
        let decline = Decline::with_default_reason(DeclineCode::NotViable);
        return ask_again(turn, table, &updated, answering, decline);
    }
    let continuation = Continuation::Resolve {
        seat: actor,
        action,
        nesting: nesting + 1,
        roll,
    };
    let offers = window.offers().len();
    let advanced = updated
        .advanced(kind, continuation, window)
        .with_more_draws(turn.draws.to_vec());
    let id = advanced.id();
    turn.on_commit.push(
        turn.pipeline_event(|scope, correlation| PipelineEvent::PendingOpened {
            scope,
            correlation,
            pending: id,
            kind,
            offers,
        }),
    );
    let mut staged = Staged::new();
    staged.replace.push(advanced);
    match apply::commit(turn, table, staged) {
        Ok(()) => {
            turn.pending = Some(id);
            Outcome::Pending(kind)
        }
        Err(decline) => Outcome::Declined(decline),
    }
}

/// The answer was not taken: the dice drawn for it are kept and the seat that gave it is asked
/// again, so the pending stays open and nothing is drawn twice.
fn ask_again(
    turn: &mut Turn<'_, '_>,
    table: &mut PipelineTable,
    updated: &PendingResolution,
    answering: SeatId,
    decline: Decline,
) -> Outcome {
    turn.on_commit.clear();
    let window = updated.window().reasked(answering);
    let continuation = match updated.continuation() {
        Continuation::Resolve {
            seat,
            action,
            nesting,
            ..
        } => Continuation::Resolve {
            seat: *seat,
            action: action.clone(),
            nesting: *nesting,
            roll: Vec::new(),
        },
        other => other.clone(),
    };
    let kept = updated
        .advanced(updated.kind(), continuation, window)
        .with_more_draws(turn.draws.to_vec());
    let mut staged = Staged::new();
    staged.replace.push(kept);
    match apply::commit(turn, table, staged) {
        Ok(()) => {
            turn.pending = Some(updated.id());
            Outcome::Declined(decline)
        }
        Err(refused) => Outcome::Declined(refused),
    }
}
