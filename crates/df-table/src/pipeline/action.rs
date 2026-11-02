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
//! a decline ends the action; a harm that needs the seat's yes opens a `pending{choice}` before
//! the machine sees anything; a ruling is reused when the stored key and scope are equal, settled
//! at once by a reviewed automatic policy, or asked of the host as a `pending{ruling}`
//! (`AwaitingAdjudicator`); a resolution that awaits answers steps the machine with that pending
//! so it reports `Waiting{pending_id}` (C.4 phase one). What the rules owner resolved (its
//! effects and typed facts) is committed with the machine's own effects (RES18).
//!
//! It never reads player text and never draws a die except through DECIDE. Nothing changes until
//! `commit`: the staged changes are made together or not at all, and only in memory; the host's
//! commit is what makes them durable.

use std::borrow::Cow;

use crate::effects::{TableEffect, TableState};
use crate::envelope::PendingId;
use crate::event::{FailureCause, PipelineStep, TableEvent};
use crate::identity::{SceneId, SeatId};
use crate::machine::{
    Decision, Machine, MachineFailureCause, MachineInput, StepInput, StepOutcome, StepResult,
};
use crate::machine_slot::MachineSlot;
use crate::outcome::{Decline, DeclineCode, Outcome, OutcomeKind, PendingKind};
use crate::pending::window::{Completion, ResolutionWindow};
use crate::pending::{
    Continuation, PendingOpening, PendingResolution, RulingSource, ScopedRuling, StoredDraw,
};
use crate::ports::{
    ActionId, Awaiting, AwaitingKind, ConfirmationReason, Draw, Offer, PlanSoFar, PortName,
    RulingAnswer, RulingKey, RulingOption, RulingRequest, RulingScope,
};

use super::admit::{self, Reading};
use super::apply::{self, ClarificationChange, RulingChange, Staged, StagedRequest};
use super::check::{self, Checked, Settled};
use super::decide::{self, Decided};
use super::events::{ClarifyCause, PipelineEvent};
use super::table_access::PipelineTable;
use super::turn::Turn;
use super::{Clarification, narrate};

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
}

/// The machine's state and the staged changes while a plan runs. The state is borrowed from the
/// snapshot the turn started from until a step changes it.
pub(super) struct PlanState<'s> {
    pub(super) slot: Cow<'s, MachineSlot>,
    pub(super) staged: Staged,
}

/// Why a plan stopped at an action: it was declined, it opened a pending, or it needs a pick.
enum Stop {
    Declined(Decline),
    Pending(PendingKind),
    Clarify(Clarification),
}

/// How one action ended: applied, or the reason the plan stops here.
type ActionEnd = Result<(), Stop>;

fn stop_declined(decline: Decline) -> ActionEnd {
    Err(Stop::Declined(decline))
}

fn stop_pending(kind: PendingKind) -> ActionEnd {
    Err(Stop::Pending(kind))
}

fn stop_clarify(clarification: Clarification) -> ActionEnd {
    Err(Stop::Clarify(clarification))
}

/// What a ruling question came to.
enum RulingFlow {
    /// Go on with this answer; `store` is a ruling to keep once the action is applied.
    Use {
        answer: RulingAnswer,
        store: Option<ScopedRuling>,
    },
    Opened,
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
    let mut state = PlanState {
        slot: Cow::Borrowed(running.slot),
        staged: Staged::new(),
    };
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
    let wants_hand_off = machine.wants_hand_off(&state.slot);
    if let Err(decline) = apply::commit(turn, table, state.staged) {
        return Outcome::Declined(decline);
    }
    if let (true, Some(Stop::Declined(_))) = (applied > 0, &stop) {
        turn.emit(
            &turn.pipeline_event(|scope, correlation| PipelineEvent::PlanPartlyApplied {
                scope,
                correlation,
                applied,
                total: plan.len(),
            }),
        );
    }
    if applied > 0 {
        turn.hand_off_wanted = wants_hand_off.unwrap_or(false);
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
                return open_confirmation(turn, table, state, seat, action, reason);
            }
            Checked::Ruling { request, draws } => {
                match settle_ruling(turn, table, state, seat, action, &request) {
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
                return await_answers(
                    turn, table, facts, machine, state, seat, action, awaiting, &drawn,
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
fn machine_broke_contract(turn: &Turn<'_, '_>) -> ActionEnd {
    turn.emit(&TableEvent::MachineFailed {
        scope: turn.scope.clone(),
        correlation: turn.correlation,
        cause: MachineFailureCause::StateInvalid,
    });
    stop_declined(Decline::with_default_reason(DeclineCode::NotViable))
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

/// Opens a pending and stages it with the commit.
#[allow(
    clippy::too_many_arguments,
    reason = "everything a pending is opened with, none of it optional"
)]
fn open_pending(
    turn: &mut Turn<'_, '_>,
    table: &PipelineTable,
    state: &mut PlanState<'_>,
    seat: SeatId,
    kind: PendingKind,
    continuation: Continuation,
    window: ResolutionWindow,
    draws: Vec<StoredDraw>,
) {
    let id = table.pending().next_id();
    let offers = window.offers().len();
    let pending = PendingResolution::open(PendingOpening {
        id,
        kind,
        opened_by: seat,
        operation: turn.correlation.operation_id,
        turn: turn.correlation.turn_seq,
        now_ms: turn.now(),
        scene: table.active_scene(),
        continuation,
        draws,
        window,
    });
    turn.pending = Some(id);
    turn.on_commit.push(
        turn.pipeline_event(|scope, correlation| PipelineEvent::PendingOpened {
            scope,
            correlation,
            pending: id,
            kind,
            offers,
        }),
    );
    state.staged.open.push(pending);
}

/// The seat's own yes is asked before the machine sees the action (C.5, RES26).
fn open_confirmation(
    turn: &mut Turn<'_, '_>,
    table: &PipelineTable,
    state: &mut PlanState<'_>,
    seat: SeatId,
    action: &ActionId,
    reason: ConfirmationReason,
) -> ActionEnd {
    let choices = &turn.pipeline.choices;
    let offer = Offer {
        seat,
        choices: vec![choices.confirm.clone(), choices.cancel.clone()],
    };
    match ResolutionWindow::new(vec![offer], Completion::AnyOffer) {
        Ok(window) => {
            let continuation = Continuation::Confirm {
                seat,
                action: action.clone(),
                reason,
            };
            open_pending(
                turn,
                table,
                state,
                seat,
                PendingKind::Choice,
                continuation,
                window,
                Vec::new(),
            );
            stop_pending(PendingKind::Choice)
        }
        Err(_) => rules_gave_garbage(turn),
    }
}

/// An `Awaiting` the pending book cannot hold is the rules owner breaking its contract: recorded
/// once, never guessed at.
fn rules_gave_garbage(turn: &Turn<'_, '_>) -> ActionEnd {
    turn.boundary_failed(
        PortName::RulesOwner,
        FailureCause::InvalidAnswer,
        turn.started_ms,
    );
    stop_declined(Decline::with_default_reason(DeclineCode::NotSupportedYet))
}

/// Phase one of a window, a choice or a roll (C.4): the machine is stepped with the pending it
/// waits on, and the pending is committed with the machine's `Waiting` state.
#[allow(
    clippy::too_many_arguments,
    reason = "the stage context of one action plus what it awaits"
)]
fn await_answers(
    turn: &mut Turn<'_, '_>,
    table: &PipelineTable,
    facts: &TableState,
    machine: &dyn Machine,
    state: &mut PlanState<'_>,
    seat: SeatId,
    action: &ActionId,
    awaiting: Awaiting,
    drawn: &[Draw],
) -> ActionEnd {
    let (kind, completion, offers, roll) = match awaiting.kind {
        AwaitingKind::Roll => {
            if awaiting.draws.is_empty() {
                return rules_gave_garbage(turn);
            }
            let offer = Offer {
                seat,
                choices: vec![turn.pipeline.choices.roll.clone()],
            };
            let sides = awaiting
                .draws
                .iter()
                .map(|request| request.sides.get())
                .collect();
            (PendingKind::Roll, Completion::AnyOffer, vec![offer], sides)
        }
        AwaitingKind::Choice => (
            PendingKind::Choice,
            Completion::EveryOffer,
            awaiting.offers,
            Vec::new(),
        ),
        AwaitingKind::Reaction => (
            PendingKind::Reaction,
            Completion::EveryOffer,
            awaiting.offers,
            Vec::new(),
        ),
    };
    let Ok(window) = ResolutionWindow::new(offers, completion) else {
        return rules_gave_garbage(turn);
    };
    let id = table.pending().next_id();
    let decision = match kind {
        PendingKind::Roll => Decision::RollPending(id),
        PendingKind::Choice | PendingKind::Reaction | PendingKind::Ruling => Decision::Pending(id),
    };
    let step = StepInput {
        correlation: turn.correlation,
        now_ms: turn.now(),
        table: facts,
        input: MachineInput::Intent {
            seat,
            action: action.clone(),
        },
        decision: Some(decision),
    };
    let result = match machine.step(&state.slot, &step) {
        Ok(result) => result,
        Err(error) => {
            turn.emit(&error.event(&turn.scope, turn.correlation));
            return stop_declined(Decline::with_default_reason(DeclineCode::NotViable));
        }
    };
    match &result.outcome {
        StepOutcome::Waiting(reported) if *reported == id => {}
        StepOutcome::Settled(Outcome::Declined(decline)) => {
            return stop_declined(decline.clone());
        }
        StepOutcome::Waiting(_) | StepOutcome::Settled(_) => {
            return machine_broke_contract(turn);
        }
    }
    absorb(turn, machine, state, result);
    let continuation = Continuation::Resolve {
        seat,
        action: action.clone(),
        nesting: 0,
        roll,
    };
    let stored: Vec<StoredDraw> = drawn.iter().copied().map(StoredDraw::of).collect();
    open_pending(turn, table, state, seat, kind, continuation, window, stored);
    stop_pending(kind)
}

/// A ruling question: reuse a stored ruling with an equal key and scope, settle it by a reviewed
/// automatic policy, or ask the host (RES15, RES48, K.4).
fn settle_ruling(
    turn: &mut Turn<'_, '_>,
    table: &PipelineTable,
    state: &mut PlanState<'_>,
    seat: SeatId,
    action: &ActionId,
    request: &RulingRequest,
) -> Result<RulingFlow, Decline> {
    let scene = table.active_scene();
    match table.rulings().find(&request.key, request.scope, scene) {
        Some(stored) if !stored.needs_reconfirmation(scene) => {
            let answer = stored.answer().clone();
            turn.ruling_note = Some(answer.clone());
            let (key, tier) = (request.key.clone(), stored.tier());
            turn.on_commit
                .push(
                    turn.pipeline_event(|scope, correlation| PipelineEvent::RulingReused {
                        scope,
                        correlation,
                        key,
                        tier,
                    }),
                );
            Ok(RulingFlow::Use {
                answer,
                store: None,
            })
        }
        Some(stored) => {
            let mut asked = request.clone();
            asked.suggested = RulingOption {
                answer: stored.answer().clone(),
                tier: stored.tier(),
            };
            open_ruling(turn, table, state, seat, action, asked, true)?;
            Ok(RulingFlow::Opened)
        }
        None if request.reviewed_automatic => {
            let answer = request.suggested.answer.clone();
            let ruling = ScopedRuling::new(
                request.key.clone(),
                request.scope,
                scene,
                answer.clone(),
                request.suggested.tier,
                RulingSource::ReviewedAutomatic,
                action.clone(),
            );
            Ok(RulingFlow::Use {
                answer,
                store: Some(ruling),
            })
        }
        None => {
            open_ruling(turn, table, state, seat, action, request.clone(), false)?;
            Ok(RulingFlow::Opened)
        }
    }
}

/// Opens a `pending{ruling}`: the host answers with a tap, the requester may cancel.
fn open_ruling(
    turn: &mut Turn<'_, '_>,
    table: &PipelineTable,
    state: &mut PlanState<'_>,
    seat: SeatId,
    action: &ActionId,
    request: RulingRequest,
    reuse: bool,
) -> Result<(), Decline> {
    let choices = &turn.pipeline.choices;
    let mut host_choices = vec![
        choices.accept_suggestion.clone(),
        choices.decline_ruling.clone(),
    ];
    // A ruling answer is a name under the same rule as an action, so none is ever skipped.
    host_choices.extend(
        request
            .alternatives
            .iter()
            .filter_map(|option| ActionId::new(option.answer.as_str()).ok()),
    );
    let mut offers = Vec::new();
    match table.host() {
        Some(host) if host == seat => {
            host_choices.push(choices.cancel_ruling.clone());
            offers.push(Offer {
                seat: host,
                choices: host_choices,
            });
        }
        Some(host) => {
            offers.push(Offer {
                seat: host,
                choices: host_choices,
            });
            offers.push(Offer {
                seat,
                choices: vec![choices.cancel_ruling.clone()],
            });
        }
        None => offers.push(Offer {
            seat,
            choices: vec![choices.cancel_ruling.clone()],
        }),
    }
    let window = ResolutionWindow::new(offers, Completion::AnyOffer).map_err(|_| {
        turn.boundary_failed(
            PortName::RulesOwner,
            FailureCause::InvalidAnswer,
            turn.started_ms,
        );
        Decline::with_default_reason(DeclineCode::NotSupportedYet)
    })?;
    let continuation = Continuation::Ruling {
        seat,
        action: action.clone(),
        request,
        reuse,
    };
    open_pending(
        turn,
        table,
        state,
        seat,
        PendingKind::Ruling,
        continuation,
        window,
        Vec::new(),
    );
    Ok(())
}
